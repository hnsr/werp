#pragma once
#include "backend.h"
#include <QMainWindow>
#include <QTimer>
#include <QSettings>
class QComboBox;
class QLabel;
class QProgressBar;
class QPushButton;
class QGroupBox;

class ConversionWindow : public QMainWindow {
    Q_OBJECT
public:
    explicit ConversionWindow(const QString &backend, const QString &file, const QStringList &backendArguments = {}, const QString &settingsFile = {}, bool discoverOnStart = true);
protected:
    bool eventFilter(QObject *watched,QEvent *event) override;
    void closeEvent(QCloseEvent *event) override;
private:
    enum class Activity { Selecting, Starting, Working, Cancelling, Finished, Closing };
    void setupUi();
    void connectSignals(bool discoverOnStart);
    void discover();
    void preview();
    void startConversion();
    void refreshActions();
    QJsonObject targetParams() const;
    QString m_file;
    QSettings m_settings;
    QComboBox *m_devices;
    QPushButton *m_refresh, *m_convert;
    QLabel *m_discoveryStatus;
    bool m_discovering = false, m_previewReady = false;
    Activity m_activity = Activity::Selecting;
    quint64 m_previewGeneration = 0;
    void updateState(const QJsonObject &state);
    void finish(const QJsonObject &state);
    void fail(const QString &message);
    void cancel();
    Backend *m_backend;
    struct FormatFields { QLabel *container, *video, *resolution, *audio; };
    QGroupBox *createFormatSection(const QString &title, const QString &name, FormatFields &fields);
    void showFormat(const QJsonObject &media, const FormatFields &fields);
    FormatFields m_source, m_target;
    QLabel *m_status, *m_output, *m_outputCaption, *m_warnings, *m_countdown;
    QProgressBar *m_progress;
    QPushButton *m_button;
    QTimer m_autoCloseTimer;
    bool m_autoClose = true;
    int m_seconds = 5;
    qint64 m_operation = 0;
    bool m_canClose = false;
};
