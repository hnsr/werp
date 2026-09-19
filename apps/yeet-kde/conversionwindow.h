#pragma once
#include "backend.h"
#include <QMainWindow>
#include <QTimer>
class QLabel;
class QProgressBar;
class QPushButton;

class ConversionWindow : public QMainWindow {
    Q_OBJECT
public:
    explicit ConversionWindow(const QString &backend, const QString &file, const QStringList &backendArguments = {});
protected:
    void closeEvent(QCloseEvent *event) override;
private:
    void updateState(const QJsonObject &state);
    void finish(const QJsonObject &state);
    void fail(const QString &message);
    void cancel();
    Backend *m_backend;
    QLabel *m_source, *m_target, *m_status, *m_output, *m_warnings, *m_countdown;
    QProgressBar *m_progress;
    QPushButton *m_button;
    QTimer m_timer;
    qint64 m_operation = 0;
    int m_seconds = 5;
    bool m_finished = false, m_cancelling = false, m_closing = false, m_canClose = false;
};
