#pragma once
#include "backend.h"
#include <QMainWindow>
#include <QJsonObject>
class QComboBox; class QLabel; class QPushButton; class QSlider; class QProgressBar; class QStackedWidget;
class Window : public QMainWindow {
    Q_OBJECT
public:
    explicit Window(const QString &backend, const QString &file = {}, bool discover = true, const QStringList &backendArguments = {});
    void openFile(const QString &file);
protected:
    void closeEvent(QCloseEvent *event) override;
private:
    void inspect();
    void discover();
    void startPlayback(bool resume);
    void stopPlayback();
    void handleEvent(const QJsonObject &message);
    void refreshActions();
    void showError(const QString &message);
    bool check(const QJsonObject &reply);
    void sendControl(const QString &method, QJsonObject params = {});
    Backend m_backend;
    QString m_file, m_directory;
    qint64 m_session = 0;
    int m_generation = 0;
    double m_duration = 0, m_resume = -1;
    bool m_inspected = false, m_busy = false, m_paused = false, m_closing = false, m_dragging = false;
    bool m_autoDiscover;
    bool m_discovering = false, m_canClose = false;
    QStackedWidget *m_pages;
    QLabel *m_fileLabel, *m_error, *m_prepareLabel, *m_time;
    QComboBox *m_devices, *m_subtitles;
    QPushButton *m_open, *m_refresh, *m_browse, *m_start, *m_resumeButton, *m_pause, *m_stop, *m_cancel, *m_retry;
    QProgressBar *m_progress;
    QSlider *m_seek;
};
