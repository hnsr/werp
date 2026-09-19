#include "../window.h"
#include <QComboBox>
#include <QFile>
#include <QJsonDocument>
#include <QProgressBar>
#include <QPushButton>
#include <QSignalSpy>
#include <QSlider>
#include <QStackedWidget>
#include <QTemporaryDir>
#include <QTest>
#include <QDir>
class WindowTest : public QObject {
    Q_OBJECT
    static QList<QJsonObject> calls(const QString &path) {
        QFile file(path); if (!file.open(QIODevice::ReadOnly)) return {};
        QList<QJsonObject> result;
        for (const auto &line : file.readAll().split('\n')) if (!line.isEmpty()) result << QJsonDocument::fromJson(line).object();
        return result;
    }
    static QList<QJsonObject> starts(const QString &path) {
        QList<QJsonObject> result;
        for (const auto &call : calls(path)) if (call["method"]=="start") result << call["params"].toObject();
        return result;
    }
    static void screenshot(Window &window,const QString &name) {
        const auto output=qEnvironmentVariable("YEET_UI_SCREENSHOTS");
        if (!output.isEmpty()) { QDir().mkpath(output); QVERIFY(window.grab().save(output+"/"+name+".png")); }
    }
private slots:
    void explicit_choices_preparation_controls_and_stop() {
        QTemporaryDir dir;
        const auto helper=dir.filePath("backend.py");
        QVERIFY(QFile::copy(QStringLiteral(YEET_TEST_BACKEND),helper));
        QVERIFY(QFile::setPermissions(helper,QFile::ReadOwner|QFile::WriteOwner|QFile::ExeOwner));
        Window window(helper,dir.filePath("test video.mkv")); window.show();
        auto *devices=window.findChild<QComboBox *>("devices");
        auto *subtitles=window.findChild<QComboBox *>("subtitles");
        auto *yeet=window.findChild<QPushButton *>("yeet");
        auto *resume=window.findChild<QPushButton *>("resume");
        auto *pages=window.findChild<QStackedWidget *>("pages");
        auto *progress=window.findChild<QProgressBar *>("preparationProgress");
        auto *pause=window.findChild<QPushButton *>("pause");
        auto *seek=window.findChild<QSlider *>("seek");
        const auto log=helper+".log";
        QTRY_COMPARE_WITH_TIMEOUT(devices->count(),3,5000);
        QTRY_COMPARE(subtitles->count(),4);
        QCOMPARE(devices->currentIndex(),0); QCOMPARE(subtitles->currentIndex(),0);
        QVERIFY(!yeet->isEnabled()); QVERIFY(resume->isVisible()); QVERIFY(starts(log).isEmpty());
        screenshot(window,"selection");
        devices->setCurrentIndex(2); subtitles->setCurrentIndex(2); QVERIFY(yeet->isEnabled());
        QTest::mouseClick(resume,Qt::LeftButton);
        QTRY_COMPARE(progress->value(),42); QCOMPARE(pages->currentIndex(),1);
        QVERIFY(!devices->isEnabled()); QVERIFY(!subtitles->isEnabled()); screenshot(window,"preparing");
        QTRY_COMPARE(starts(log).size(),1);
        QCOMPARE(starts(log).last()["position"].toDouble(),25.0);
        QCOMPARE(starts(log).last()["subtitles"].toObject()["index"].toInt(),3);
        QTRY_COMPARE(pages->currentIndex(),2); screenshot(window,"playing");
        QTest::mouseClick(pause,Qt::LeftButton); QTRY_COMPARE(pause->text(),QString("Play"));
        seek->setValue(50000); QVERIFY(QMetaObject::invokeMethod(seek,"sliderReleased",Qt::DirectConnection));
        QTest::mouseClick(pause,Qt::LeftButton); QTRY_COMPARE(pause->text(),QString("Pause"));
        QTest::mouseClick(window.findChild<QPushButton *>("stop"),Qt::LeftButton);
        QTRY_COMPARE(pages->currentIndex(),0); QTRY_VERIFY(yeet->isEnabled()); QCOMPARE(subtitles->currentIndex(),2);
        QTest::mouseClick(yeet,Qt::LeftButton); QTRY_COMPARE(starts(log).size(),2);
        QCOMPARE(starts(log).last()["position"].toDouble(),0.0);
        QTRY_VERIFY(window.findChild<QPushButton *>("cancel")->isEnabled());
        QTest::mouseClick(window.findChild<QPushButton *>("cancel"),Qt::LeftButton);
        QTRY_COMPARE(pages->currentIndex(),0);
        bool sawSeek=false;
        for (const auto &call : calls(log)) if (call["method"]=="seek") { QCOMPARE(call["params"].toObject()["position"].toDouble(),60.0); sawSeek=true; }
        QVERIFY(sawSeek);
        QSignalSpy exited(window.findChild<Backend *>(),&Backend::exited);
        window.close(); QTRY_COMPARE_WITH_TIMEOUT(exited.count(),1,5000); QVERIFY(!window.isVisible());
    }
    void real_helper_handshake_and_readonly_inspection() {
        QTemporaryDir dir;
        const auto file=dir.filePath("video.mp4"); QFile video(file); QVERIFY(video.open(QIODevice::WriteOnly)); video.write("source"); video.close();
        const auto probe=dir.filePath("probe");
        QFile script(probe); QVERIFY(script.open(QIODevice::WriteOnly));
        script.write("#!/bin/sh\ncat \""); script.write(YEET_TEST_METADATA); script.write("\"\n"); script.close();
        QVERIFY(QFile::setPermissions(probe,QFile::ReadOwner|QFile::WriteOwner|QFile::ExeOwner));
        const auto wrapper=dir.filePath("helper");
        QFile launch(wrapper); QVERIFY(launch.open(QIODevice::WriteOnly));
        launch.write("#!/bin/sh\nexec \""); launch.write(YEET_REAL_HELPER); launch.write("\" --no-inhibit-sleep --ffprobe \""); launch.write(probe.toUtf8()); launch.write("\"\n"); launch.close();
        QVERIFY(QFile::setPermissions(wrapper,QFile::ReadOwner|QFile::WriteOwner|QFile::ExeOwner));
        Window window(wrapper,file,false); window.show();
        QTRY_VERIFY_WITH_TIMEOUT(window.findChild<QComboBox *>("subtitles")->isEnabled(),5000);
        QVERIFY(!window.findChild<QPushButton *>("yeet")->isEnabled());
        QCOMPARE(window.findChild<QStackedWidget *>("pages")->currentIndex(),0);
        QSignalSpy exited(window.findChild<Backend *>(),&Backend::exited);
        window.close(); QTRY_COMPARE_WITH_TIMEOUT(exited.count(),1,5000);
    }
};
QTEST_MAIN(WindowTest)
#include "window-test.moc"
