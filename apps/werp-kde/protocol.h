#pragma once

#include <QJsonArray>
#include <QJsonObject>
#include <QString>

// The helper speaks JSON, but windows should only handle its display data.
// Keep success/error framing and device eligibility in one place.
namespace Protocol {

struct Reply {
    explicit Reply(const QJsonObject &message) : message(message) {}

    bool ok() const { return message.value("ok").toBool(); }
    QJsonObject result() const { return message.value("result").toObject(); }
    QString error() const {
        return message.value("error").toObject().value("message").toString();
    }
    QString errorCode() const {
        return message.value("error").toObject().value("code").toString();
    }
    qint64 resultId(const char *field) const { return result().value(QLatin1String(field)).toInteger(); }

private:
    QJsonObject message;
};

struct Device {
    explicit Device(const QJsonObject &value)
        : id(value.value("id").toString()), name(value.value("name").toString()),
          model(value.value("model").toString()),
          audioOnly(value.value("capabilities").isDouble()
                    && !(value.value("capabilities").toInt() & 1)),
          hasAddress(!value.value("addresses").toArray().isEmpty()) {}

    QString label() const { return name + " · " + model; }
    QString id;
    QString name;
    QString model;
    bool audioOnly;
    bool hasAddress;
};

struct Inspection {
    explicit Inspection(const QJsonObject &value)
        : media(value.value("media").toObject()),
          subtitles(value.value("subtitles").toArray()),
          suggestedSubtitles(value.value("suggested_subtitles").toObject()),
          subtitleWarning(value.value("subtitle_warning").toString()),
          resumePosition(value.value("resume_position")),
          resumeWarning(value.value("resume_warning").toString()) {}

    QJsonObject media;
    QJsonArray subtitles;
    QJsonObject suggestedSubtitles;
    QString subtitleWarning;
    QJsonValue resumePosition;
    QString resumeWarning;
};

struct SubtitleChoice {
    explicit SubtitleChoice(const QJsonObject &value)
        : kind(value.value("kind").toString()), index(value.value("index").toInt()),
          path(value.value("path").toString()), codec(value.value("codec").toString()),
          language(value.value("language").toString()), title(value.value("title").toString()),
          forced(value.value("forced").toBool()), burnIn(value.value("burn_in").toBool()),
          supported(value.value("supported").toBool()) {}

    QString kind;
    int index;
    QString path;
    QString codec;
    QString language;
    QString title;
    bool forced;
    bool burnIn;
    bool supported;
};

inline QJsonObject fileParams(const QString &file) { return {{"file", file}}; }
inline QJsonObject sessionParams(qint64 id) { return {{"session_id", id}}; }
inline QJsonObject operationParams(qint64 id) { return {{"operation_id", id}}; }
inline QJsonObject conversionParams(const QString &file, const QString &deviceId) {
    auto params=fileParams(file);
    if (!deviceId.isEmpty()) params.insert("device_id",deviceId);
    return params;
}
inline QJsonObject startParams(const QString &file, const QString &deviceId,
                               const QJsonObject &subtitles, int subtitleDelayMs, double position) {
    return {{"file",file}, {"device_id",deviceId}, {"subtitles",subtitles},
            {"subtitle_delay_ms",subtitleDelayMs}, {"position",position}};
}

} // namespace Protocol
