#pragma once
#include "backend.h"
#include <functional>

class GuiSettings {
public:
    explicit GuiSettings(const QString &settingsFile = {});
    void load(Backend &backend, std::function<void(const QString &)> done);
    void saveLastDevice(Backend &backend, const QString &deviceId,
                        std::function<void(const QString &)> done = {});
    QString lastDeviceId() const { return m_lastDeviceId; }
    bool conversionAutoClose() const { return m_conversionAutoClose; }
private:
    void apply(const QJsonObject &reply);
    QString m_path, m_lastDeviceId;
    bool m_conversionAutoClose = true;
};
