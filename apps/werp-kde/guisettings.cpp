#include "guisettings.h"
#include "protocol.h"
#include <QStandardPaths>

GuiSettings::GuiSettings(const QString &settingsFile) {
    const QString directory = QStandardPaths::writableLocation(QStandardPaths::GenericConfigLocation)+"/werp/";
    m_path = settingsFile.isEmpty() ? directory+"gui.toml" : settingsFile;
}

void GuiSettings::apply(const QJsonObject &reply) {
    const auto result = Protocol::Reply(reply).result();
    m_lastDeviceId = result["last_device_id"].toString();
    m_conversionAutoClose = result["conversion_auto_close"].toBool(true);
}

void GuiSettings::load(Backend &backend, std::function<void(const QString &)> done) {
    backend.request("get_gui_preferences",{{"path",m_path}},[this,done=std::move(done)](const QJsonObject &reply) {
        const Protocol::Reply response(reply);
        if (response.ok()) apply(reply);
        if (done) done(response.ok() ? QString() : response.error());
    });
}

void GuiSettings::saveLastDevice(Backend &backend, const QString &deviceId,
                                 std::function<void(const QString &)> done) {
    backend.request("set_gui_last_device",{{"path",m_path},{"device_id",deviceId}},
                    [this,done=std::move(done)](const QJsonObject &reply) {
        const Protocol::Reply response(reply);
        if (response.ok()) apply(reply);
        if (done) done(response.ok() ? QString() : response.error());
    });
}
