#include "conversionwindow.h"
#include <KLocalizedString>
#include <QCloseEvent>
#include <QFileInfo>
#include <QJsonArray>
#include <QGroupBox>
#include <QFormLayout>
#include <QLabel>
#include <QProgressBar>
#include <QPushButton>
#include <QShortcut>
#include <QSettings>
#include <QStandardPaths>
#include <QVBoxLayout>

namespace {
void bold(QWidget *widget) {
    auto font=widget->font(); font.setBold(true); widget->setFont(font);
}
QString containerName(const QString &container) {
    const auto names=container.split(',');
    if (names.contains("mp4")) return QStringLiteral("MP4");
    if (names.contains("matroska")) return QStringLiteral("Matroska (MKV)");
    return container.toUpper();
}
QString codecName(const QJsonObject &stream) {
    auto codec=stream["codec"].toString(i18n("Unknown codec")).toUpper();
    if (codec=="H264") codec="H.264";
    const auto profile=stream["profile"].toString();
    if (!profile.isEmpty()) codec += " · "+profile;
    return codec;
}
QLabel *label(QWidget *parent, const char *name, const QString &text={}) {
    auto *result=new QLabel(text,parent); result->setObjectName(name);
    result->setTextFormat(Qt::PlainText); result->setWordWrap(true);
    result->setTextInteractionFlags(Qt::TextSelectableByMouse);
    return result;
}
}
ConversionWindow::ConversionWindow(const QString &backend,const QString &file,const QStringList &arguments,const QString &settingsFile) {
    QSettings settings(settingsFile.isEmpty() ? QStandardPaths::writableLocation(QStandardPaths::GenericConfigLocation)+"/yeet/kde-ui.ini" : settingsFile,QSettings::IniFormat);
    m_autoClose=settings.value("conversion/autoClose",true).toBool();
    setWindowTitle(i18n("Convert only")); resize(740,480);
    auto *central=new QWidget(this); setCentralWidget(central);
    auto *layout=new QVBoxLayout(central); layout->setSpacing(12);
    auto *selected=new QHBoxLayout;
    auto *selectedCaption=label(central,"conversionFileLabel",i18n("Selected video:"));
    bold(selectedCaption); selectedCaption->setWordWrap(false);
    selected->addWidget(selectedCaption,0,Qt::AlignTop);
    auto *filename=label(central,"conversionFile",QFileInfo(file).fileName());
    filename->setToolTip(file); selected->addWidget(filename,1); layout->addLayout(selected);
    auto *formats=new QHBoxLayout;
    formats->setSpacing(16);
    formats->addWidget(createFormatSection(i18n("Source"),"conversionSource",m_source),1);
    formats->addWidget(createFormatSection(i18n("Target"),"conversionTarget",m_target),1);
    layout->addLayout(formats);
    layout->addWidget(label(central,"conversionNote",i18n("Compatible streams are copied. The original file is kept; subtitles remain with the original.")));
    auto *outputLayout=new QVBoxLayout; outputLayout->setSpacing(2);
    m_outputCaption=label(central,"conversionOutputLabel",i18n("Available file:"));
    bold(m_outputCaption); m_outputCaption->hide(); outputLayout->addWidget(m_outputCaption);
    m_output=label(central,"conversionOutput"); outputLayout->addWidget(m_output); layout->addLayout(outputLayout);
    m_warnings=label(central,"conversionWarnings"); layout->addWidget(m_warnings);
    layout->addStretch();
    m_status=label(central,"conversionStatus",i18n("Starting backend…")); layout->addWidget(m_status);
    m_progress=new QProgressBar(central); m_progress->setObjectName("conversionProgress"); m_progress->setRange(0,0); layout->addWidget(m_progress);
    auto *bottom=new QHBoxLayout;
    m_countdown=label(central,"conversionCountdown");
    m_countdown->setWordWrap(false);
    m_countdown->setSizePolicy(QSizePolicy::MinimumExpanding,QSizePolicy::Preferred);
    bottom->addWidget(m_countdown,1);
    m_button=new QPushButton(i18n("Cancel"),central); m_button->setObjectName("conversionButton"); bottom->addWidget(m_button); layout->addLayout(bottom);
    auto *quit=new QShortcut(QKeySequence::Quit,this); connect(quit,&QShortcut::activated,this,&QWidget::close);
    connect(m_button,&QPushButton::clicked,this,[this] { if (m_finished) close(); else cancel(); });
    m_autoCloseTimer.setInterval(1000);
    m_autoCloseTimer.setTimerType(Qt::PreciseTimer);
    connect(&m_autoCloseTimer,&QTimer::timeout,this,[this] {
        if (--m_seconds<=0) { m_autoCloseTimer.stop(); close(); }
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
QGroupBox *ConversionWindow::createFormatSection(const QString &title,const QString &name,FormatFields &fields) {
    auto *section=new QGroupBox(centralWidget()); section->setObjectName(name);
    auto *sectionLayout=new QVBoxLayout(section);
    // Breeze centres native group-box titles regardless of their alignment.
    // A heading inside the frame gives all styles the same left alignment.
    auto *heading=label(section,qPrintable(name+"Heading"),title);
    heading->setAlignment(Qt::AlignLeft|Qt::AlignVCenter); bold(heading);
    sectionLayout->addWidget(heading);
    auto *form=new QFormLayout; sectionLayout->addLayout(form);
    form->setFieldGrowthPolicy(QFormLayout::AllNonFixedFieldsGrow);
    form->setLabelAlignment(Qt::AlignLeft|Qt::AlignTop);
    auto row=[&](const QString &caption,const char *suffix) {
        auto *value=label(section,qPrintable(name+suffix),i18n("Determining…"));
        auto valueFont=font(); valueFont.setBold(false); value->setFont(valueFont);
        // Keep data at regular weight, independently of heading styling.
        auto *captionLabel=label(section,qPrintable(name+suffix+"Label"),caption);
        bold(captionLabel); captionLabel->setWordWrap(false);
        form->addRow(captionLabel,value); return value;
    };
    fields.container=row(i18n("Container:"),"Container");
    fields.video=row(i18n("Video:"),"Video");
    fields.resolution=row(i18n("Resolution:"),"Resolution");
    fields.audio=row(i18n("Audio:"),"Audio");
    return section;
}
void ConversionWindow::showFormat(const QJsonObject &media,const FormatFields &fields) {
    fields.container->setText(containerName(media["container"].toString()));
    QStringList videos,audio,resolutions;
    for (const auto &value : media["streams"].toArray()) {
        const auto stream=value.toObject();
        if (stream["attached_picture"].toBool()) continue;
        if (stream["kind"]=="video") {
            videos << codecName(stream);
            QString resolution=i18n("Determining…");
            if (stream["width"].toInt()>0 && stream["height"].toInt()>0) {
                resolution=QString("%1 × %2").arg(stream["width"].toInt()).arg(stream["height"].toInt());
                if (stream["frame_rate"].isDouble()) resolution += i18n(" · %1 fps",QString::number(stream["frame_rate"].toDouble(),'g',4));
            }
            resolutions << resolution;
        } else if (stream["kind"]=="audio") {
            auto description=codecName(stream);
            if (stream["channels"].toInt()>0) description += i18np(" · %1 channel"," · %1 channels",stream["channels"].toInt());
            audio << description;
        }
    }
    fields.video->setText(videos.isEmpty()?i18n("None"):videos.join("\n"));
    fields.resolution->setText(resolutions.isEmpty()?QStringLiteral("—"):resolutions.join("\n"));
    fields.audio->setText(audio.isEmpty()?i18n("None"):audio.join("\n"));
}
void ConversionWindow::updateState(const QJsonObject &state) {
    if (state["source"].isObject()) showFormat(state["source"].toObject(),m_source);
    if (state["target"].isObject()) showFormat(state["target"].toObject(),m_target);
    else if (state["planned_target"].isObject()) showFormat(state["planned_target"].toObject(),m_target);
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
        m_outputCaption->show(); m_output->setText(state["output"].toString());
        if (m_autoClose) {
            m_seconds=5;
            m_countdown->setText(i18np("Closing in %1 second…","Closing in %1 seconds…",m_seconds));
            m_autoCloseTimer.start();
        } else m_countdown->setText(i18n("Auto-close disabled"));
    }
}
void ConversionWindow::fail(const QString &message) {
    m_autoCloseTimer.stop();
    m_countdown->clear();
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
    m_closing=true; m_autoCloseTimer.stop(); m_button->setEnabled(false);
    m_status->setText(i18n("Closing and cleaning up…")); m_backend->shutdown();
}
