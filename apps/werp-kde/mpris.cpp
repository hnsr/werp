#include "mpris.h"

#include <QCoreApplication>
#include <QDBusAbstractAdaptor>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusObjectPath>
#include <QFileInfo>
#include <QUrl>
#include <QtMath>

namespace {
constexpr auto path = "/org/mpris/MediaPlayer2";
constexpr auto playerInterface = "org.mpris.MediaPlayer2.Player";

class RootAdaptor : public QDBusAbstractAdaptor {
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.mpris.MediaPlayer2")
    Q_PROPERTY(bool CanQuit READ canQuit)
    Q_PROPERTY(bool Fullscreen READ fullscreen)
    Q_PROPERTY(bool CanSetFullscreen READ canSetFullscreen)
    Q_PROPERTY(bool CanRaise READ canRaise)
    Q_PROPERTY(bool HasTrackList READ hasTrackList)
    Q_PROPERTY(QString Identity READ identity)
    Q_PROPERTY(QString DesktopEntry READ desktopEntry)
    Q_PROPERTY(QStringList SupportedUriSchemes READ supportedUriSchemes)
    Q_PROPERTY(QStringList SupportedMimeTypes READ supportedMimeTypes)
public:
    explicit RootAdaptor(Mpris *owner) : QDBusAbstractAdaptor(owner), m_owner(owner) {}
    bool canQuit() const { return true; }
    bool fullscreen() const { return false; }
    bool canSetFullscreen() const { return false; }
    bool canRaise() const { return true; }
    bool hasTrackList() const { return false; }
    QString identity() const { return QStringLiteral("Werp"); }
    QString desktopEntry() const { return QStringLiteral("nl.hnsr.Werp"); }
    QStringList supportedUriSchemes() const { return {}; }
    QStringList supportedMimeTypes() const { return {}; }
public slots:
    void Raise() { emit m_owner->raiseRequested(); }
    void Quit() { emit m_owner->quitRequested(); }
private:
    Mpris *m_owner;
};

class PlayerAdaptor : public QDBusAbstractAdaptor {
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.mpris.MediaPlayer2.Player")
    Q_PROPERTY(QString PlaybackStatus READ playbackStatus)
    Q_PROPERTY(QString LoopStatus READ loopStatus WRITE setLoopStatus)
    Q_PROPERTY(double Rate READ rate WRITE setRate)
    Q_PROPERTY(bool Shuffle READ shuffle WRITE setShuffle)
    Q_PROPERTY(QVariantMap Metadata READ metadata)
    Q_PROPERTY(double Volume READ volume WRITE setVolume)
    Q_PROPERTY(qlonglong Position READ position)
    Q_PROPERTY(double MinimumRate READ minimumRate)
    Q_PROPERTY(double MaximumRate READ maximumRate)
    Q_PROPERTY(bool CanGoNext READ canGoNext)
    Q_PROPERTY(bool CanGoPrevious READ canGoPrevious)
    Q_PROPERTY(bool CanPlay READ canPlay)
    Q_PROPERTY(bool CanPause READ canPause)
    Q_PROPERTY(bool CanSeek READ canSeek)
    Q_PROPERTY(bool CanControl READ canControl)
public:
    explicit PlayerAdaptor(Mpris *owner) : QDBusAbstractAdaptor(owner), m_owner(owner) {}
    QString playbackStatus() const { return m_owner->playbackStatus(); }
    QString loopStatus() const { return QStringLiteral("None"); }
    void setLoopStatus(const QString &) {}
    double rate() const { return 1.0; }
    void setRate(double) {}
    bool shuffle() const { return false; }
    void setShuffle(bool) {}
    QVariantMap metadata() const { return m_owner->metadata(); }
    double volume() const { return 1.0; }
    void setVolume(double) {}
    qlonglong position() const { return m_owner->position(); }
    double minimumRate() const { return 1.0; }
    double maximumRate() const { return 1.0; }
    bool canGoNext() const { return false; }
    bool canGoPrevious() const { return false; }
    bool canPlay() const { return m_owner->canPlay(); }
    bool canPause() const { return m_owner->canPause(); }
    bool canSeek() const { return m_owner->canSeek(); }
    bool canControl() const { return m_owner->canControl(); }
public slots:
    void Next() {}
    void Previous() {}
    void Pause() { if (m_owner->canPause()) emit m_owner->pauseRequested(); }
    void PlayPause() {
        if (m_owner->playbackStatus() == QLatin1String("Playing")) Pause();
        else Play();
    }
    void Stop() { if (m_owner->active()) emit m_owner->stopRequested(); }
    void Play() { if (m_owner->canPlay()) emit m_owner->playRequested(); }
    void Seek(qlonglong offset) {
        if (m_owner->canSeek()) emit m_owner->seekRequested(qMax(0.0, (m_owner->position() + offset) / 1000000.0));
    }
    void SetPosition(const QDBusObjectPath &trackId, qlonglong position) {
        if (m_owner->canSeek() && trackId.path() == m_owner->trackId() && position >= 0
            && position <= qRound64(m_owner->duration() * 1000000))
            emit m_owner->seekRequested(qMax(0.0, position / 1000000.0));
    }
    void OpenUri(const QString &) {} // No target can be chosen through MPRIS.
private:
    Mpris *m_owner;
};
}

Mpris::Mpris(QObject *parent) : QObject(parent) {
    new RootAdaptor(this);
    new PlayerAdaptor(this);
    auto bus = QDBusConnection::sessionBus();
    if (!bus.isConnected()) return;
    const auto base = QStringLiteral("org.mpris.MediaPlayer2.werp");
    const auto instance = base + QStringLiteral(".instance") + QString::number(QCoreApplication::applicationPid());
    if (bus.registerService(base)) m_service = base;
    else if (bus.registerService(instance)) m_service = instance;
    if (!m_service.isEmpty() && !bus.registerObject(QString::fromLatin1(path), this, QDBusConnection::ExportAdaptors)) {
        bus.unregisterService(m_service);
        m_service.clear();
    }
}

Mpris::~Mpris() {
    if (m_service.isEmpty()) return;
    auto bus = QDBusConnection::sessionBus();
    bus.unregisterObject(QString::fromLatin1(path));
    bus.unregisterService(m_service);
}

QVariantMap Mpris::metadata() const {
    if (m_trackNumber == 0) return {};
    QVariantMap value;
    value.insert(QStringLiteral("mpris:trackid"), QVariant::fromValue(QDBusObjectPath(m_trackId)));
    value.insert(QStringLiteral("xesam:title"), QFileInfo(m_file).fileName());
    value.insert(QStringLiteral("xesam:url"), QUrl::fromLocalFile(m_file).toString());
    if (m_duration > 0) value.insert(QStringLiteral("mpris:length"), qRound64(m_duration * 1000000));
    return value;
}

qlonglong Mpris::position() const {
    const double elapsed = m_status == QLatin1String("Playing") && m_clock.isValid()
        ? m_clock.elapsed() / 1000.0 : 0.0;
    return qRound64(qMax(0.0, m_duration > 0 ? qMin(m_duration, m_position + elapsed) : m_position + elapsed) * 1000000);
}

void Mpris::changed(const QVariantMap &properties) {
    if (m_service.isEmpty()) return;
    auto message = QDBusMessage::createSignal(QString::fromLatin1(path),
        QStringLiteral("org.freedesktop.DBus.Properties"), QStringLiteral("PropertiesChanged"));
    message.setArguments({QString::fromLatin1(playerInterface), properties, QStringList{}});
    QDBusConnection::sessionBus().send(message);
}

void Mpris::setMedia(const QString &file, double durationSeconds) {
    if (file != m_file) {
        m_file = file;
        m_trackId = file.isEmpty() ? QStringLiteral("/org/mpris/MediaPlayer2/TrackList/NoTrack")
                                    : QStringLiteral("/org/werp/MediaPlayer2/track%1").arg(++m_trackNumber);
        m_position = 0;
        m_clock.invalidate();
    }
    m_duration = qMax(0.0, durationSeconds);
    changed({{QStringLiteral("Metadata"), metadata()}, {QStringLiteral("CanSeek"), canSeek()}});
}

void Mpris::setState(const QString &status, double positionSeconds, double durationSeconds, bool control) {
    const auto oldPosition = position();
    const bool wasActive = m_active;
    m_status = status;
    m_position = qMax(0.0, positionSeconds);
    m_duration = qMax(0.0, durationSeconds);
    m_active = control;
    m_clock.restart();
    changed({{QStringLiteral("PlaybackStatus"), m_status}, {QStringLiteral("Metadata"), metadata()},
             {QStringLiteral("CanPause"), canPause()}, {QStringLiteral("CanSeek"), canSeek()}});
    if (m_service.isEmpty() || !wasActive || !control || qAbs(position() - oldPosition) <= 2000000) return;
    auto message = QDBusMessage::createSignal(QString::fromLatin1(path),
        QString::fromLatin1(playerInterface), QStringLiteral("Seeked"));
    message.setArguments({position()});
    QDBusConnection::sessionBus().send(message);
}

void Mpris::setCanPlay(bool value) {
    if (m_canPlay == value) return;
    m_canPlay = value;
    changed({{QStringLiteral("CanPlay"), value}});
}

void Mpris::clear() {
    m_status = QStringLiteral("Stopped");
    m_active = false;
    m_canPlay = false;
    m_position = 0;
    m_clock.invalidate();
    changed({{QStringLiteral("PlaybackStatus"), m_status},
             {QStringLiteral("CanPlay"), false}, {QStringLiteral("CanPause"), false},
             {QStringLiteral("CanSeek"), false}});
}

#include "mpris.moc"
