#pragma once
#include <QObject>
#include <QProcess>
#include <QJsonObject>
#include <QHash>
#include <functional>

class Backend : public QObject {
    Q_OBJECT
public:
    explicit Backend(QString program, QObject *parent = nullptr, const QStringList &arguments = {});
    void start();
    void shutdown();
    bool ready() const { return m_ready; }
    using Callback = std::function<void(const QJsonObject &)>;
    void request(const QString &method, const QJsonObject &params, Callback callback = {});
signals:
    void connected();
    void event(const QJsonObject &message);
    void failed(const QString &message);
    void exited();
private:
    void receive();
    void fail(const QString &message);
    QString m_program;
    QProcess m_process;
    QByteArray m_buffer;
    QString m_diagnostics;
    QHash<qint64, Callback> m_pending;
    qint64 m_nextId = 1;
    bool m_ready = false;
    bool m_closing = false;
};
