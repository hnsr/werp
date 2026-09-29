#pragma once

#include <QElapsedTimer>
#include <QObject>
#include <QVariantMap>

// The D-Bus adaptors live in mpris.cpp. This object owns the advertised state;
// the window updates it only from backend events, so remote clients see the
// receiver's state rather than optimistic button clicks.
class Mpris : public QObject {
    Q_OBJECT
public:
    explicit Mpris(QObject *parent = nullptr);
    ~Mpris() override;
    void setMedia(const QString &file, double durationSeconds);
    void setState(const QString &status, double positionSeconds, double durationSeconds,
                  bool canControl);
    void clear();
    QString playbackStatus() const { return m_status; }
    QVariantMap metadata() const;
    qlonglong position() const;
    bool canControl() const { return true; }
    bool canSeek() const { return m_active && m_duration > 0; }
    bool canPlay() const { return m_canPlay; }
    bool canPause() const { return m_active; }
    double duration() const { return m_duration; }
    bool active() const { return m_active; }
    QString trackId() const { return m_trackId; }
    void setCanPlay(bool value);
signals:
    void playRequested();
    void pauseRequested();
    void stopRequested();
    void seekRequested(double seconds);
    void raiseRequested();
    void quitRequested();
private:
    void changed(const QVariantMap &properties);
    QString m_service;
    QString m_file;
    QString m_trackId = QStringLiteral("/org/mpris/MediaPlayer2/TrackList/NoTrack");
    QString m_status = QStringLiteral("Stopped");
    double m_duration = 0;
    double m_position = 0;
    bool m_active = false;
    bool m_canPlay = false;
    quint64 m_trackNumber = 0;
    QElapsedTimer m_clock;
};
