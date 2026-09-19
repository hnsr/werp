#include "window.h"
#include <QApplication>
#include <QCommandLineParser>
#include <QFileInfo>
#include <KAboutData>
#include <KLocalizedString>
int main(int argc,char **argv) {
    QApplication app(argc,argv);
    KLocalizedString::setApplicationDomain("yeet");
    KAboutData about("yeet",i18n("Yeet"),"0.1.0");
    about.setShortDescription(i18n("Play local videos on a Chromecast"));
    KAboutData::setApplicationData(about);
    QApplication::setDesktopFileName("org.yeet.Yeet");
    QApplication::setWindowIcon(QIcon::fromTheme("video-display"));
    QCommandLineParser parser; parser.setApplicationDescription(about.shortDescription()); parser.addHelpOption(); parser.addVersionOption();
    parser.addPositionalArgument("file",i18n("Local video to open"),"[file]");
    parser.addOption({"backend",i18n("Backend helper executable"),"path"});
    parser.addOption({"no-discovery",i18n("Skip the initial device scan (development/testing)")});
    parser.addOption({"http-port",i18n("Fixed local media server port (0 chooses automatically)"),"port","0"});
    parser.process(app);
    bool validPort=false;
    const auto port=parser.value("http-port").toUInt(&validPort);
    if (!validPort || port>65535) parser.showHelp(2);
    if (parser.positionalArguments().size()>1) parser.showHelp(2);
    QString backend=parser.value("backend");
    if (backend.isEmpty()) {
        backend=QCoreApplication::applicationDirPath()+"/yeet-backend";
        if (!QFileInfo::exists(backend)) backend=QCoreApplication::applicationDirPath()+"/"+QStringLiteral(YEET_HELPER_RELATIVE);
    }
    Window window(backend,parser.positionalArguments().value(0),!parser.isSet("no-discovery"),{"--http-port",QString::number(port)});
    window.show(); return app.exec();
}
