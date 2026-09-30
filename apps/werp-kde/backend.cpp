#include "backend.h"
#include <QJsonDocument>
#include <QTimer>

Backend::Backend(QString program, QObject *parent, const QStringList &arguments) : QObject(parent), m_program(std::move(program)) {
    m_process.setArguments(arguments);
    m_handshakeTimer.setSingleShot(true);
    m_handshakeTimer.setInterval(5000);
    connect(&m_handshakeTimer, &QTimer::timeout, this, [this] {
        if (!m_ready && !m_closing && m_process.state()==QProcess::Running) {
            fail(tr("The backend did not complete its handshake.")); shutdown();
        }
    });
    connect(&m_process, &QProcess::readyReadStandardOutput, this, &Backend::receive);
    connect(&m_process, &QProcess::readyReadStandardError, this, [this] {
        m_diagnostics += QString::fromUtf8(m_process.readAllStandardError());
        m_diagnostics = m_diagnostics.right(4096);
    });
    connect(&m_process, &QProcess::started, this, [this] {
        request("hello", {{"version", 2}}, [this](const QJsonObject &reply) {
            if (reply["ok"].toBool() && reply["result"].toObject()["version"].toInt() == 2) {
                m_handshakeTimer.stop();
                m_ready = true;
                emit connected();
            } else { fail(tr("The backend uses an incompatible protocol.")); shutdown(); }
        });
    });
    connect(&m_process, &QProcess::errorOccurred, this, [this](QProcess::ProcessError error) {
        if (!m_closing && error == QProcess::FailedToStart) fail(tr("Could not start the backend: %1").arg(m_process.errorString()));
    });
    connect(&m_process, &QProcess::finished, this, [this](int code, QProcess::ExitStatus status) {
        m_handshakeTimer.stop();
        m_ready = false; m_pending.clear();
        if (!m_closing) fail(tr("The backend stopped unexpectedly (%1). %2").arg(
            status == QProcess::CrashExit ? tr("crashed") : QString::number(code), m_diagnostics));
        emit exited();
    });
}
Backend::~Backend() {
    // QProcess may emit finished while being destroyed. Our callback state and
    // the owning window's widgets can already be gone at that point. Normal
    // window closure still uses shutdown() and waits for graceful helper exit.
    disconnect(&m_process,nullptr,this,nullptr);
}
void Backend::start() {
    if (m_process.state() != QProcess::NotRunning) return;
    ++m_generation;
    m_buffer.clear(); m_diagnostics.clear(); m_pending.clear(); m_ready = false; m_closing = false;
    m_process.setProgram(m_program);
    m_process.start();
    m_handshakeTimer.start();
}
void Backend::request(const QString &method, const QJsonObject &params, Callback callback) {
    if (m_process.state() != QProcess::Running) {
        if (callback) callback({{"ok",false},{"error",QJsonObject{{"message",tr("Backend is unavailable.")}}}});
        return;
    }
    if (m_pending.size() >= 32 || m_process.bytesToWrite() > 65536) {
        if (callback) callback({{"ok",false},{"error",QJsonObject{{"message",tr("Too many pending requests.")}}}});
        return;
    }
    const auto id = m_nextId++;
    QJsonObject message{{"id",id},{"method",method}};
    if (!params.isEmpty()) message["params"] = params;
    m_pending.insert(id, std::move(callback));
    m_process.write(QJsonDocument(message).toJson(QJsonDocument::Compact) + '\n');
}
void Backend::receive() {
    m_buffer += m_process.readAllStandardOutput();
    if (m_buffer.size() > 8 * 1024 * 1024) { fail(tr("Backend message exceeded the size limit.")); shutdown(); return; }
    while (true) {
        const auto newline = m_buffer.indexOf('\n');
        if (newline < 0) break;
        const auto line = m_buffer.left(newline); m_buffer.remove(0,newline+1);
        QJsonParseError parse;
        const auto document = QJsonDocument::fromJson(line,&parse);
        if (parse.error != QJsonParseError::NoError || !document.isObject()) {
            fail(tr("Invalid message from the backend.")); shutdown(); return;
        }
        const auto message = document.object();
        if (message.contains("event")) { emit event(message); continue; }
        const auto id = message["id"].toInteger(-1);
        if (m_pending.contains(id)) {
            auto callback = m_pending.take(id);
            if (callback) callback(message);
        }
    }
}
void Backend::fail(const QString &message) { m_ready = false; emit failed(message); }
void Backend::shutdown() {
    m_handshakeTimer.stop();
    m_closing = true; m_ready = false;
    if (m_process.state() == QProcess::NotRunning) { emit exited(); return; }
    const auto generation=m_generation;
    request("shutdown", {});
    // EOF is also a shutdown signal if an explicit request cannot be delivered.
    m_process.closeWriteChannel();
    QTimer::singleShot(30000, this, [this,generation] {
        if (generation==m_generation && m_closing && m_process.state() != QProcess::NotRunning) m_process.terminate();
    });
    QTimer::singleShot(35000, this, [this,generation] {
        if (generation==m_generation && m_closing && m_process.state() != QProcess::NotRunning) m_process.kill();
    });
}
