#pragma once
#include "backend.h"
#include "guisettings.h"
#include "mpris.h"
#include <QMainWindow>
#include <QJsonObject>
class QShortcut; class QDragEnterEvent; class QDragMoveEvent; class QDropEvent;
class QComboBox; class QLabel; class QPushButton; class QSlider; class QProgressBar; class QStackedWidget;
class SelectedVideoPanel;
class QSpinBox; class QTimer;

class Window : public QMainWindow {
    Q_OBJECT
public:
    explicit Window(const QString &backend, const QString &file = {}, bool discover = true, const QStringList &backendArguments = {}, const QString &settingsFile = {});
    void openFile(const QString &file);
protected:
    void closeEvent(QCloseEvent *event) override;
    void dragEnterEvent(QDragEnterEvent *event) override;
    void dragMoveEvent(QDragMoveEvent *event) override;
    void dropEvent(QDropEvent *event) override;
private:
    enum class Activity { Idle, Starting, Preparing, Playing, Paused, Stopping, Closing };
    bool busy() const { return m_activity != Activity::Idle; }
    void setupUi();
    void connectSignals();
    void inspect();
    void discover();
    void loadSubtitleDelay();
    void applySubtitleChange();
    void restoreActiveSubtitles();
    void startPlayback(bool resume);
    void stopPlayback();
    void handleEvent(const QJsonObject &message);
    void refreshActions();
    void showError(const QString &message);
    bool check(const QJsonObject &reply);
    void sendControl(const QString &method, QJsonObject params = {});
    Backend m_backend;
    Mpris m_mpris;
    GuiSettings m_settings;
    QShortcut *m_togglePlayback;
    QString m_file, m_directory;
    qint64 m_session = 0;
    int m_generation = 0, m_delayGeneration = 0;
    double m_duration = 0, m_resume = -1;
    Activity m_activity = Activity::Idle;
    bool m_inspected = false, m_dragging = false, m_delayLoading = false;
    bool m_autoDiscover;
    bool m_discovering = false, m_canClose = false, m_applySuggestedSubtitle = true;
    bool m_subtitleChangeable = true, m_subtitleUpdating = false;
    QString m_activeSubtitleData, m_activeSubtitleText;
    int m_activeDelay = 0;
    QTimer *m_subtitleTimer;
    QWidget *m_choices;
    QStackedWidget *m_pages;
    SelectedVideoPanel *m_selectedVideo;
    QLabel *m_error, *m_prepareLabel, *m_time;
    QComboBox *m_devices, *m_subtitles;
    QPushButton *m_open, *m_refresh, *m_browse, *m_start, *m_resumeButton, *m_pause, *m_stop, *m_cancel, *m_retry;
    QProgressBar *m_progress, *m_discoveryProgress;
    QSlider *m_seek;
    QSpinBox *m_subtitleDelay;
};
