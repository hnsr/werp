#include "window.h"
#include "protocol.h"
#include <QSpinBox>
#include <limits>
#include "selectedvideopanel.h"
#include <QAbstractItemView>
#include <QCloseEvent>
#include <QComboBox>
#include <QDragEnterEvent>
#include <QDropEvent>
#include <QMimeData>
#include <QMimeDatabase>
#include <QShortcut>
#include <QStandardPaths>
#include <QUrl>
#include <QFileDialog>
#include <QFileInfo>
#include <QFormLayout>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QJsonDocument>
#include <QLabel>
#include <QProgressBar>
#include <QPushButton>
#include <QScreen>
#include <QSlider>
#include <QStackedWidget>
#include <QStandardItemModel>
#include <QTimer>
#include <QVBoxLayout>
#include <KLocalizedString>
#include <algorithm>
#include <cmath>

// Keep the field within the window, but let the popup fit long track names.
class SubtitleComboBox : public QComboBox {
public:
    SubtitleComboBox() {
        setSizePolicy(QSizePolicy::Expanding,QSizePolicy::Fixed);
        setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
        setMinimumContentsLength(30);
        view()->setTextElideMode(Qt::ElideNone);
        view()->setHorizontalScrollBarPolicy(Qt::ScrollBarAsNeeded);
        connect(this,&QComboBox::currentTextChanged,this,&QWidget::setToolTip);
    }
    void showPopup() override {
        int contentWidth=width();
        for (int i=0; i<count(); ++i)
            contentWidth=qMax(contentWidth,fontMetrics().horizontalAdvance(itemText(i))+48);
        view()->setMinimumWidth(qMin(contentWidth,screen()->availableGeometry().width()-32));
        QComboBox::showPopup();
    }
};
static void addSubtitle(QComboBox *combo,const QString &text,const QString &data) {
    combo->addItem(text,data);
    combo->setItemData(combo->count()-1,text,Qt::ToolTipRole);
}
static QString timestamp(double seconds) {
    const auto n = qMax(0, static_cast<int>(seconds));
    return QString("%1:%2:%3").arg(n/3600).arg((n/60)%60,2,10,QChar('0')).arg(n%60,2,10,QChar('0'));
}
static QString payload(const QJsonObject &value) { return QString::fromUtf8(QJsonDocument(value).toJson(QJsonDocument::Compact)); }
Window::Window(const QString &backend, const QString &file, bool discoverOnStart, const QStringList &backendArguments, const QString &settingsFile)
    : m_backend(backend,this,backendArguments),
      m_settings(settingsFile.isEmpty() ? QStandardPaths::writableLocation(QStandardPaths::GenericConfigLocation)+"/werp/kde-ui.ini" : settingsFile,QSettings::IniFormat),
      m_autoDiscover(discoverOnStart) {
    setupUi();
    connectSignals();
    if (!file.isEmpty()) openFile(file);
    refreshActions();
    m_backend.start();
}
void Window::setupUi() {
    setWindowTitle(i18n("Werp"));
    resize(640,440);
    setAcceptDrops(true);

    auto *central = new QWidget(this);
    auto *layout = new QVBoxLayout(central);
    setWindowSpacing(layout);
    setCentralWidget(central);
    m_selectedVideo = new SelectedVideoPanel(central);
    layout->addWidget(m_selectedVideo);
    m_open = new QPushButton(QIcon::fromTheme("document-open"),i18n("Open video…"),central);
    m_open->setObjectName("openVideo");
    layout->addWidget(m_open,0,Qt::AlignRight);
    m_error = new QLabel(this);
    m_error->setTextFormat(Qt::PlainText);
    m_error->setWordWrap(true);
    m_error->setObjectName("message");
    m_error->hide();
    layout->addWidget(m_error);
    m_retry = new QPushButton(i18n("Reconnect backend"),this);
    m_retry->hide();
    layout->addWidget(m_retry);
    m_pages = new QStackedWidget(this);
    m_pages->setObjectName("pages");
    layout->addWidget(m_pages,1);

    auto *startup = new QWidget;
    auto *startupLayout = new QVBoxLayout(startup);
    startupLayout->setContentsMargins(0,0,0,0);
    auto *form = new QFormLayout;
    form->setFieldGrowthPolicy(QFormLayout::AllNonFixedFieldsGrow);
    startupLayout->addLayout(form);
    m_devices = new QComboBox;
    m_devices->setObjectName("devices");
    m_devices->addItem(i18n("Choose a device…"),"");
    m_refresh = new QPushButton(QIcon::fromTheme("view-refresh"),i18n("Refresh"));
    auto *deviceRow = new QHBoxLayout;
    deviceRow->addWidget(m_devices,1);
    deviceRow->addWidget(m_refresh);
    form->addRow(i18n("Device:"),deviceRow);
    m_discoveryProgress = new QProgressBar;
    m_discoveryProgress->setObjectName("discoveryProgress");
    m_discoveryProgress->setRange(0,0);
    m_discoveryProgress->setTextVisible(false);
    m_discoveryProgress->setFixedHeight(6);
    m_discoveryProgress->setAccessibleName(i18n("Searching for devices"));
    auto discoveryPolicy=m_discoveryProgress->sizePolicy();
    discoveryPolicy.setRetainSizeWhenHidden(true);
    m_discoveryProgress->setSizePolicy(discoveryPolicy);
    form->addRow("",m_discoveryProgress);
    m_subtitles = new SubtitleComboBox;
    m_subtitles->setObjectName("subtitles");
    m_subtitles->addItem(i18n("None"),payload({{"kind","none"}}));
    form->addRow(i18n("Subtitles:"),m_subtitles);
    m_browse = new QPushButton(i18n("Choose subtitle file…"));
    form->addRow("",m_browse);
    m_subtitleDelay = new QSpinBox;
    m_subtitleDelay->setObjectName("subtitleDelay");
    m_subtitleDelay->setRange(std::numeric_limits<int>::min(),std::numeric_limits<int>::max());
    m_subtitleDelay->setSingleStep(100);
    m_subtitleDelay->setToolTip(i18n("Positive values show subtitles later; negative values show them earlier. Applies when playback starts."));
    auto *delayRow = new QHBoxLayout;
    delayRow->addWidget(m_subtitleDelay);
    delayRow->addWidget(new QLabel(i18n("ms")));
    delayRow->addStretch();
    form->addRow(i18n("Subtitle delay:"),delayRow);
    auto *buttons = new QHBoxLayout;
    m_start = new QPushButton(i18n("Cast"));
    m_start->setObjectName("werp");
    m_start->setDefault(true);
    m_resumeButton = new QPushButton(i18n("Cast from last position"));
    m_resumeButton->setObjectName("resume");
    auto *quit = new QPushButton(i18n("Quit"));
    quit->setObjectName("quit");
    quit->setToolTip(i18n("Quit (Ctrl+Q)"));
    connect(quit,&QPushButton::clicked,this,&QWidget::close);
    auto *quitShortcut = new QShortcut(QKeySequence(Qt::CTRL | Qt::Key_Q),this);
    quitShortcut->setAutoRepeat(false);
    connect(quitShortcut,&QShortcut::activated,this,&QWidget::close);
    buttons->addWidget(m_start,1);
    buttons->addWidget(m_resumeButton,1);
    buttons->addWidget(quit);
    startupLayout->addStretch();
    startupLayout->addLayout(buttons);
    m_pages->addWidget(startup);

    auto *preparing = new QWidget;
    auto *prepareLayout = new QVBoxLayout(preparing);
    prepareLayout->setContentsMargins(0,0,0,0);
    m_prepareLabel = new QLabel(i18n("Preparing playback…"));
    m_prepareLabel->setTextFormat(Qt::PlainText);
    m_prepareLabel->setWordWrap(true);
    m_progress = new QProgressBar;
    m_progress->setObjectName("preparationProgress");
    m_progress->setRange(0,0);
    m_cancel = new QPushButton(i18n("Cancel"));
    m_cancel->setObjectName("cancel");
    prepareLayout->addStretch();
    prepareLayout->addWidget(m_prepareLabel);
    prepareLayout->addWidget(m_progress);
    prepareLayout->addWidget(m_cancel);
    m_pages->addWidget(preparing);

    auto *playing = new QWidget;
    auto *playerLayout = new QVBoxLayout(playing);
    playerLayout->setContentsMargins(0,0,0,0);
    m_time = new QLabel;
    m_time->setAlignment(Qt::AlignCenter);
    m_seek = new QSlider(Qt::Horizontal);
    m_seek->setRange(0,100000);
    m_seek->setObjectName("seek");
    m_pause = new QPushButton(i18n("Pause"));
    m_pause->setObjectName("pause");
    m_stop = new QPushButton(i18n("Stop"));
    m_stop->setObjectName("stop");
    auto *controls = new QHBoxLayout;
    controls->addStretch();
    controls->addWidget(m_pause);
    controls->addWidget(m_stop);
    controls->addStretch();
    playerLayout->addStretch();
    playerLayout->addWidget(m_time);
    playerLayout->addWidget(m_seek);
    playerLayout->addLayout(controls);
    m_pages->addWidget(playing);
    m_togglePlayback = new QShortcut(QKeySequence(Qt::Key_Space),this);
    m_togglePlayback->setAutoRepeat(false);
    m_togglePlayback->setEnabled(false);
}
void Window::connectSignals() {
    connect(m_togglePlayback,&QShortcut::activated,m_pause,&QPushButton::click);
    m_pause->setToolTip(i18n("Pause or play (Space)"));
    connect(m_open,&QPushButton::clicked,this,[this] {
        const auto path = QFileDialog::getOpenFileName(this,i18n("Open video"),m_directory,i18n("Videos (*.mp4 *.mkv *.webm *.avi *.mov *.m4v *.ts);;All files (*)"));
        if (!path.isEmpty()) openFile(path);
    });
    connect(m_browse,&QPushButton::clicked,this,[this] {
        const auto path = QFileDialog::getOpenFileName(this,i18n("Choose subtitles"),m_directory,i18n("Subtitles (*.srt *.vtt *.ass *.ssa);;All files (*)"));
        if (path.isEmpty()) return;
        addSubtitle(m_subtitles,i18n("External: %1",QFileInfo(path).fileName()),payload({{"kind","external"},{"path",path}}));
        m_subtitles->setCurrentIndex(m_subtitles->count()-1);
    });
    connect(m_refresh,&QPushButton::clicked,this,&Window::discover);
    connect(m_retry,&QPushButton::clicked,&m_backend,&Backend::start);
    connect(m_devices,&QComboBox::currentIndexChanged,this,&Window::refreshActions);
    connect(m_start,&QPushButton::clicked,this,[this] { startPlayback(false); });
    connect(m_resumeButton,&QPushButton::clicked,this,[this] { startPlayback(true); });
    connect(m_cancel,&QPushButton::clicked,this,&Window::stopPlayback);
    connect(m_stop,&QPushButton::clicked,this,&Window::stopPlayback);
    connect(m_pause,&QPushButton::clicked,this,[this] {
        sendControl(m_activity == Activity::Paused ? "play" : "pause");
    });
    connect(m_seek,&QSlider::sliderPressed,this,[this] { m_dragging=true; });
    connect(m_seek,&QSlider::sliderReleased,this,[this] {
        m_dragging=false;
        if (m_duration > 0) {
            const double position=std::min(m_duration-0.1,m_duration*m_seek->value()/100000.0);
            sendControl("seek",{{"position",position}});
        }
    });
    connect(&m_backend,&Backend::connected,this,[this] {
        m_retry->hide();
        showError({});
        if (!m_file.isEmpty()) inspect();
        if (m_autoDiscover) discover();
        refreshActions();
    });
    connect(&m_backend,&Backend::event,this,&Window::handleEvent);
    connect(&m_backend,&Backend::failed,this,[this](const QString &message) {
        if (m_activity == Activity::Closing) return;
        m_activity=Activity::Idle;
        m_session=0;
        m_discovering=false;
        m_togglePlayback->setEnabled(false);
        m_pages->setCurrentIndex(0);
        m_selectedVideo->setActiveSubtitle({});
        showError(message);
        m_retry->show();
        refreshActions();
    });
    connect(&m_backend,&Backend::exited,this,[this] {
        if (m_activity != Activity::Closing) return;
        m_canClose=true;
        QTimer::singleShot(0,this,&Window::close);
    });
}
void Window::openFile(const QString &file) {
    if (busy()) { showError(i18n("Stop playback before opening another video.")); return; }
    ++m_generation; m_file=QFileInfo(file).absoluteFilePath(); m_directory=QFileInfo(m_file).absolutePath();
    m_selectedVideo->setFile(m_file);
    m_selectedVideo->setActiveSubtitle({});
    m_subtitleDelay->setValue(0);
    m_inspected=false; m_applySuggestedSubtitle=true; m_resume=-1; m_duration=0; showError({});
    m_subtitles->clear(); m_subtitles->addItem(i18n("None"),payload({{"kind","none"}}));
    if (m_backend.ready()) inspect();
    refreshActions();
}
bool Window::check(const QJsonObject &reply) {
    const Protocol::Reply response(reply);
    if (response.ok()) return true;
    showError(response.error().isEmpty() ? i18n("The operation failed.") : response.error());
    return false;
}
void Window::showError(const QString &message) { m_error->setText(message); m_error->setVisible(!message.isEmpty()); }
void Window::inspect() {
    m_inspected=false; refreshActions();
    const auto generation=++m_generation; const auto file=m_file;
    m_backend.request("inspect",Protocol::fileParams(file),[this,generation](const QJsonObject &reply) {
        if (generation!=m_generation || m_activity == Activity::Closing) return;
        if (!check(reply)) { m_inspected=false; refreshActions(); return; }
        const Protocol::Inspection result(Protocol::Reply(reply).result());
        const auto media=result.media;
        m_duration=media["duration_seconds"].toDouble(); m_directory=QFileInfo(media["path"].toString()).absolutePath();
        m_resume=result.resumePosition.isDouble() ? result.resumePosition.toDouble() : -1;
        const auto selected=m_applySuggestedSubtitle && !result.suggestedSubtitles.isEmpty()
            ? payload(result.suggestedSubtitles) : m_subtitles->currentData().toString();
        m_subtitles->clear(); m_subtitles->addItem(i18n("None"),payload({{"kind","none"}}));
        for (const auto value : result.subtitles) {
            const Protocol::SubtitleChoice choice(value.toObject());
            QJsonObject data;
            QString text;
            if (choice.kind=="embedded") {
                data={{"kind","embedded"},{"index",choice.index}};
                text=i18n("Track %1 · %2 · %3",choice.index,
                          choice.language.isEmpty() ? i18n("Unknown language") : choice.language,
                          choice.codec);
                if (!choice.title.isEmpty()) text+=" · "+choice.title;
                if (choice.forced) text+=i18n(" · forced");
                if (choice.burnIn) text+=i18n(" · requires video conversion");
            } else {
                data={{"kind","external"},{"path",choice.path}};
                text=i18n("External: %1",QFileInfo(choice.path).fileName());
            }
            addSubtitle(m_subtitles,text,payload(data));
            if (!choice.supported) {
                if (auto *model=qobject_cast<QStandardItemModel *>(m_subtitles->model())) model->item(m_subtitles->count()-1)->setEnabled(false);
            }
        }
        const int previous=m_subtitles->findData(selected);
        if (previous>=0) m_subtitles->setCurrentIndex(previous);
        else {
            const auto previousChoice=QJsonDocument::fromJson(selected.toUtf8()).object();
            if (previousChoice["kind"]=="external") {
                addSubtitle(m_subtitles,i18n("External: %1",QFileInfo(previousChoice["path"].toString()).fileName()),selected);
                m_subtitles->setCurrentIndex(m_subtitles->count()-1);
            }
        }
        if (m_applySuggestedSubtitle && !result.subtitleWarning.isEmpty())
            showError(i18n("Choose subtitles manually: %1",result.subtitleWarning));
        m_applySuggestedSubtitle=false;
        m_inspected=true;
        if (!result.resumeWarning.isEmpty()) showError(i18n("Saved position unavailable: %1",result.resumeWarning));
        refreshActions();
    });
}
void Window::discover() {
    m_discovering=true;
    refreshActions();
    m_backend.request("discover",{},[this](const QJsonObject &reply) {
        if (m_activity == Activity::Closing) return;
        m_discovering=false;
        refreshActions(); if (!check(reply)) return;
        const auto previous=m_devices->currentData().toString();
        m_devices->clear(); m_devices->addItem(i18n("Choose a device…"),"");
        for (const auto value : Protocol::Reply(reply).result()["devices"].toArray()) {
            const Protocol::Device device(value.toObject());
            QString text=device.label();
            if (device.audioOnly) text+=i18n(" (audio only)");
            m_devices->addItem(text,device.id);
            if (device.audioOnly || !device.hasAddress) {
                if (auto *model=qobject_cast<QStandardItemModel *>(m_devices->model())) model->item(m_devices->count()-1)->setEnabled(false);
            }
        }
        m_settings.sync();
        for (const auto &candidate : {previous,m_settings.value("lastDeviceId").toString()}) {
            const int index=m_devices->findData(candidate);
            if (index>0 && (m_devices->model()->flags(m_devices->model()->index(index,0)) & Qt::ItemIsEnabled)) {
                m_devices->setCurrentIndex(index); break;
            }
        }
        if (m_devices->count()==1) showError(i18n("No devices found. Check the network and try Refresh."));
        refreshActions();
    });
}
void Window::refreshActions() {
    const bool idle=m_activity == Activity::Idle && m_backend.ready();
    m_discoveryProgress->setVisible(m_discovering);
    m_devices->setItemText(0,m_discovering ? i18n("Searching for devices…") : i18n("Choose a device…"));
    m_subtitleDelay->setEnabled(idle && m_inspected);
    m_open->setEnabled(m_activity == Activity::Idle); m_devices->setEnabled(idle && !m_discovering); m_subtitles->setEnabled(idle && m_inspected);
    m_browse->setEnabled(idle && m_inspected);
    const bool canStart=idle && !m_discovering && m_inspected && !m_devices->currentData().toString().isEmpty();
    m_start->setEnabled(canStart); m_resumeButton->setVisible(m_resume>=0); m_resumeButton->setEnabled(canStart && m_resume>=0);
    m_resumeButton->setToolTip(i18n("Resume at %1",timestamp(m_resume)));
    m_refresh->setEnabled(idle && !m_discovering); m_seek->setEnabled((m_activity == Activity::Playing || m_activity == Activity::Paused) && m_duration>0);
}
void Window::startPlayback(bool resume) {
    if (!m_inspected || busy() || m_devices->currentData().toString().isEmpty()) return;
    m_selectedVideo->setActiveSubtitle({});
    m_activity=Activity::Starting; showError({}); m_prepareLabel->setText(i18n("Preparing playback…")); m_progress->setRange(0,0);
    m_cancel->setEnabled(false); m_pages->setCurrentIndex(1); refreshActions();
    const auto deviceId=m_devices->currentData().toString();
    const auto subtitle=QJsonDocument::fromJson(m_subtitles->currentData().toString().toUtf8()).object();
    const auto params=Protocol::startParams(m_file,deviceId,subtitle,m_subtitleDelay->value(),resume?m_resume:0.0);
    m_backend.request("start",params,[this,deviceId](const QJsonObject &reply) {
        if (m_activity == Activity::Closing) return;
        if (!check(reply)) { m_activity=Activity::Idle; m_pages->setCurrentIndex(0); refreshActions(); return; }
        m_settings.setValue("lastDeviceId",deviceId); m_settings.sync();
        if (m_settings.status()!=QSettings::NoError) showError(i18n("Could not remember the selected device."));
        m_session=Protocol::Reply(reply).resultId("session_id"); m_cancel->setEnabled(true); m_stop->setEnabled(true);
    });
}
void Window::sendControl(const QString &method,QJsonObject params) {
    if (!m_session) return;
    const auto session=m_session;
    params["session_id"]=m_session;
    m_backend.request(method,params,[this,session](const QJsonObject &reply) { if (m_session==session) check(reply); });
}
void Window::stopPlayback() {
    if (!m_session) return;
    m_activity=Activity::Stopping;
    m_togglePlayback->setEnabled(false);
    m_stop->setEnabled(false); m_cancel->setEnabled(false); m_pause->setEnabled(false); m_seek->setEnabled(false);
    m_backend.request("stop",Protocol::sessionParams(m_session),[this](const QJsonObject &reply) { check(reply); });
}
void Window::handleEvent(const QJsonObject &message) {
    if (m_activity == Activity::Closing) return;
    if (message["session_id"].toInteger()!=m_session || !m_session) return;
    if (message["event"]=="ended") {
        m_activity=Activity::Idle; m_session=0; m_togglePlayback->setEnabled(false); m_pages->setCurrentIndex(0); m_pause->setEnabled(true); m_selectedVideo->setActiveSubtitle({});
        if (!message["error"].isNull()) showError(message["error"].toString());
        inspect(); refreshActions(); return;
    }
    const auto state=message["state"].toObject(); const auto phase=state["phase"].toString();
    if (m_activity == Activity::Stopping && (phase=="playing" || phase=="paused" || phase=="buffering")) return;
    if (phase=="playing" || phase=="paused" || (phase=="buffering" && m_pages->currentIndex()==2)) {
        m_selectedVideo->setActiveSubtitle(m_subtitles->currentText());
        m_togglePlayback->setEnabled(m_stop->isEnabled());
        m_activity=phase=="paused" ? Activity::Paused : Activity::Playing;
        m_pages->setCurrentIndex(2); m_pause->setText(m_activity == Activity::Paused ? i18n("Play") : i18n("Pause"));
        const auto position=state["position_seconds"].toDouble(); m_duration=state["duration_seconds"].toDouble(m_duration);
        m_seek->setEnabled(m_duration>0 && m_stop->isEnabled());
        m_time->setText(timestamp(position)+" / "+timestamp(m_duration)+(phase=="buffering"?i18n(" · Buffering"):QString()));
        if (!m_dragging && m_duration>0) m_seek->setValue(static_cast<int>(100000*position/m_duration));
    } else if (phase=="preparing" || phase=="connecting" || phase=="loading") {
        m_activity=Activity::Preparing;
        m_togglePlayback->setEnabled(false);
        m_pages->setCurrentIndex(1);
        const auto operation=state["preparation_operation"].toString();
        m_prepareLabel->setText(state["message"].toString(operation.isEmpty()?i18n("Starting playback…"):operation));
        if (state["preparation_fraction"].isDouble()) { m_progress->setRange(0,100); m_progress->setValue(qRound(100*state["preparation_fraction"].toDouble())); }
        else m_progress->setRange(0,0);
    } else if (phase=="stopping") { m_activity=Activity::Stopping; m_togglePlayback->setEnabled(false); m_prepareLabel->setText(i18n("Stopping and cleaning up…")); m_cancel->setEnabled(false); }
}
static QString droppedVideo(const QMimeData *data) {
    const auto urls=data->urls();
    if (urls.size()!=1 || !urls.first().isLocalFile()) return {};
    const auto path=urls.first().toLocalFile();
    if (!QFileInfo(path).isFile()) return {};
    const auto mime=QMimeDatabase().mimeTypeForFile(path,QMimeDatabase::MatchExtension).name();
    return mime.startsWith("video/") ? path : QString();
}
void Window::dragEnterEvent(QDragEnterEvent *event) {
    if (!busy() && event->possibleActions().testFlag(Qt::CopyAction) && !droppedVideo(event->mimeData()).isEmpty()) { event->setDropAction(Qt::CopyAction); event->accept(); }
    else event->ignore();
}
void Window::dragMoveEvent(QDragMoveEvent *event) {
    if (!busy() && event->possibleActions().testFlag(Qt::CopyAction) && !droppedVideo(event->mimeData()).isEmpty()) { event->setDropAction(Qt::CopyAction); event->accept(); }
    else event->ignore();
}
void Window::dropEvent(QDropEvent *event) {
    const auto path=droppedVideo(event->mimeData());
    if (busy() || path.isEmpty() || !event->possibleActions().testFlag(Qt::CopyAction)) { event->ignore(); return; }
    openFile(path); event->setDropAction(Qt::CopyAction); event->accept();
}
void Window::closeEvent(QCloseEvent *event) {
    if (m_canClose) { event->accept(); return; }
    event->ignore();
    if (m_activity == Activity::Closing) return;
    m_activity=Activity::Closing; setEnabled(false); m_backend.shutdown();
}
