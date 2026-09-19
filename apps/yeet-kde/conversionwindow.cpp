#include "conversionwindow.h"
#include <KLocalizedString>
#include <QCloseEvent>
#include <QFileInfo>
#include <QJsonArray>
#include <QLabel>
#include <QProgressBar>
#include <QPushButton>
#include <QShortcut>
#include <QVBoxLayout>

namespace {
QString format(const QJsonObject &media) {
    QStringList parts{media["container"].toString()};
    for (const auto &value : media["streams"].toArray()) {
        const auto stream=value.toObject();
        const auto kind=stream["kind"].toString();
        if (kind!="video" && kind!="audio") continue;
        if (stream["attached_picture"].toBool()) continue;
        QString description=stream["codec"].toString(i18n("Unknown codec")).toUpper();
        if (kind=="video") {
            if (stream["profile"].isString()) description += " "+stream["profile"].toString();
            if (stream["width"].toInt()>0) description += QString(" · %1×%2").arg(stream["width"].toInt()).arg(stream["height"].toInt());
            if (stream["pixel_format"].isString()) description += " · "+stream["pixel_format"].toString();
        } else if (stream["channels"].toInt()>0) description += i18np(" · %1 channel"," · %1 channels",stream["channels"].toInt());
        parts << description;
    }
    return parts.join("\n");
}
QLabel *label(QWidget *parent, const char *name, const QString &text={}) {
    auto *result=new QLabel(text,parent); result->setObjectName(name);
    result->setTextFormat(Qt::PlainText); result->setWordWrap(true);
    result->setTextInteractionFlags(Qt::TextSelectableByMouse);
    return result;
}
}
ConversionWindow::ConversionWindow(const QString &backend,const QString &file,const QStringList &arguments) {
    setWindowTitle(i18n("Yeet — Convert only")); resize(620,460);
    auto *central=new QWidget(this); setCentralWidget(central);
    auto *layout=new QVBoxLayout(central); layout->setSpacing(12);
    auto *filename=label(central,"conversionFile",i18n("Selected video: %1",QFileInfo(file).fileName()));
    filename->setToolTip(file); layout->addWidget(filename);
    m_source=label(central,"conversionSource",i18n("Source: Inspecting…")); layout->addWidget(m_source);
    m_target=label(central,"conversionTarget",i18n("Target: Determining…")); layout->addWidget(m_target);
    layout->addWidget(label(central,"conversionNote",i18n("Compatible streams are copied. The original file is kept; subtitles remain with the original.")));
    m_output=label(central,"conversionOutput"); layout->addWidget(m_output);
    m_warnings=label(central,"conversionWarnings"); layout->addWidget(m_warnings);
    layout->addStretch();
    m_status=label(central,"conversionStatus",i18n("Starting backend…")); layout->addWidget(m_status);
    m_progress=new QProgressBar(central); m_progress->setObjectName("conversionProgress"); m_progress->setRange(0,0); layout->addWidget(m_progress);
    auto *bottom=new QHBoxLayout;
    m_countdown=label(central,"conversionCountdown"); bottom->addWidget(m_countdown); bottom->addStretch();
    m_button=new QPushButton(i18n("Cancel"),central); m_button->setObjectName("conversionButton"); bottom->addWidget(m_button); layout->addLayout(bottom);
    auto *quit=new QShortcut(QKeySequence::Quit,this); connect(quit,&QShortcut::activated,this,&QWidget::close);
    connect(m_button,&QPushButton::clicked,this,[this] { if (m_finished) close(); else cancel(); });
    m_timer.setInterval(1000);
    connect(&m_timer,&QTimer::timeout,this,[this] {
        if (--m_seconds<=0) { m_timer.stop(); close(); }
        else m_countdown->setText(i18np("Closing in %1 second…","Closing in %1 seconds…",m_seconds));
    });
    m_backend=new Backend(backend,this,arguments);
    connect(m_backend,&Backend::connected,this,[this,file] {
        if (m_closing || m_finished || m_cancelling) return;
        m_status->setText(i18n("Inspecting media…"));
        m_backend->request("convert",{{"file",QFileInfo(file).absoluteFilePath()}},[this](const QJsonObject &reply) {
            if (m_closing || m_finished) return;
            if (!reply["ok"].toBool()) { fail(reply["error"].toObject()["message"].toString()); return; }
            m_operation=reply["result"].toObject()["operation_id"].toInteger();
        });
    });
    connect(m_backend,&Backend::event,this,[this](const QJsonObject &message) {
        if (m_closing || m_finished || !m_operation || message["operation_id"].toInteger()!=m_operation) return;
        const auto event=message["event"].toString();
        if (event=="conversion_state") updateState(message["state"].toObject());
        else if (event=="conversion_ended") finish(message["state"].toObject());
    });
    connect(m_backend,&Backend::failed,this,[this](const QString &error) { if (!m_closing) fail(error); });
    connect(m_backend,&Backend::exited,this,[this] {
        if (m_closing) { m_canClose=true; QTimer::singleShot(0,this,&QWidget::close); }
    });
    m_backend->start();
}
void ConversionWindow::updateState(const QJsonObject &state) {
    if (state["source"].isObject()) m_source->setText(i18n("Source:\n%1",format(state["source"].toObject())));
    if (state["target_description"].isString()) m_target->setText(i18n("Target: %1",state["target_description"].toString()));
    if (state["target"].isObject()) m_target->setText(i18n("Target:\n%1",format(state["target"].toObject())));
    if (!m_cancelling) m_status->setText(state["message"].toString());
    if (state["fraction"].isDouble()) { m_progress->setRange(0,100); m_progress->setValue(qBound(0,qRound(state["fraction"].toDouble()*100),100)); }
    else m_progress->setRange(0,0);
    QStringList warnings; for (const auto &warning : state["warnings"].toArray()) warnings << warning.toString();
    m_warnings->setText(warnings.join("\n"));
}
void ConversionWindow::finish(const QJsonObject &state) {
    m_cancelling=false; updateState(state); m_finished=true; m_operation=0;
    m_progress->setRange(0,100); m_button->setText(i18n("Close")); m_button->setEnabled(true);
    if (state["error"].isString()) m_status->setText(state["error"].toString());
    if (state["phase"].toString()=="completed") {
        m_progress->setValue(100);
        m_output->setText(i18n("Available file:\n%1",state["output"].toString()));
        m_countdown->setText(i18np("Closing in %1 second…","Closing in %1 seconds…",m_seconds)); m_timer.start();
    }
}
void ConversionWindow::fail(const QString &message) {
    m_timer.stop(); m_countdown->clear();
    finish({{"phase","failed"},{"error",message}});
    m_backend->shutdown(); // Also cleans up if the error was a protocol failure.
}
void ConversionWindow::cancel() {
    m_cancelling=true; m_button->setEnabled(false); m_status->setText(i18n("Cancelling and cleaning up…"));
    if (!m_operation) {
        // Before the start acknowledgement, shutdown still cancels a queued job.
        // Wait for helper exit before claiming cleanup is finished.
        connect(m_backend,&Backend::exited,this,[this] {
            if (!m_closing) finish({{"phase","cancelled"},{"message",i18n("Cancelled; cleanup completed")}});
        });
        m_backend->shutdown(); return;
    }
    m_backend->request("cancel_conversion",{{"operation_id",m_operation}},[this](const QJsonObject &reply) {
        // A terminal event may already be in flight when cancellation is sent.
        if (!reply["ok"].toBool() && reply["error"].toObject()["code"]!="invalid_operation" && !m_finished && !m_closing)
            fail(reply["error"].toObject()["message"].toString());
    });
}
void ConversionWindow::closeEvent(QCloseEvent *event) {
    if (m_canClose) { event->accept(); return; }
    event->ignore();
    if (m_closing) return;
    m_closing=true; m_timer.stop(); m_button->setEnabled(false);
    m_status->setText(i18n("Closing and cleaning up…")); m_backend->shutdown();
}
