#include "guisettings.h"
#include "protocol.h"

GuiSettings::GuiSettings(const QString &settingsFile) : m_path(settingsFile) {}

void GuiSettings::apply(const QJsonObject &reply) {
    const auto result = Protocol::Reply(reply).result();
    m_lastDeviceId = result["last_device_id"].toString();
    m_conversionAutoClose = result["conversion_auto_close"].toBool(true);
}

void GuiSettings::load(Backend &backend, std::function<void(const QString &)> done) {
    QJsonObject params{{"path",m_path.isEmpty() ? QJsonValue(QJsonValue::Null) : QJsonValue(m_path)}};
    backend.request("get_gui_preferences",params,[this,done=std::move(done)](const QJsonObject &reply) {
        const Protocol::Reply response(reply);
        if (response.ok()) apply(reply);
        if (done) done(response.ok() ? QString() : response.error());
    });
}

void GuiSettings::saveLastDevice(Backend &backend, const QString &deviceId,
                                 std::function<void(const QString &)> done) {
    backend.request("set_gui_last_device",{{"path",m_path.isEmpty() ? QJsonValue(QJsonValue::Null) : QJsonValue(m_path)},
                    {"device_id",deviceId}},
                    [this,done=std::move(done)](const QJsonObject &reply) {
        const Protocol::Reply response(reply);
        if (response.ok()) apply(reply);
        if (done) done(response.ok() ? QString() : response.error());
    });
}
