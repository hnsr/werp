#include "../window.h"
#include "../conversionwindow.h"
#include <QLabel>
#include <QGroupBox>
#include <QRegularExpression>
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
#include <QDragEnterEvent>
#include <QDropEvent>
#include <QMimeData>
#include <QUrl>
#include <QShortcut>
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
    static void screenshot(QWidget &window,const QString &name) {
        const auto output=qEnvironmentVariable("YEET_UI_SCREENSHOTS");
        if (!output.isEmpty()) { QDir().mkpath(output); QVERIFY(window.grab().save(output+"/"+name+".png")); }
    }
private slots:
    void destroying_active_backend_does_not_call_destroyed_callback_state() {
        QTemporaryDir dir; const auto helper=dir.filePath("backend.py");
        QVERIFY(QFile::copy(QStringLiteral(YEET_TEST_BACKEND),helper));
        QVERIFY(QFile::setPermissions(helper,QFile::ReadOwner|QFile::WriteOwner|QFile::ExeOwner));
        auto backend=std::make_unique<Backend>(helper);
        backend->start(); QTRY_VERIFY(backend->ready());
        QTest::ignoreMessage(QtWarningMsg,QRegularExpression("QProcess: Destroyed while process .* is still running."));
        backend.reset(); // Emergency destruction, as when a test assertion exits early.
    }
    void conversion_progress_completion_stays_open() {
        QTemporaryDir dir; const auto helper=dir.filePath("backend.py");
        QVERIFY(QFile::copy(QStringLiteral(YEET_TEST_BACKEND),helper));
        QVERIFY(QFile::setPermissions(helper,QFile::ReadOwner|QFile::WriteOwner|QFile::ExeOwner));
        const auto settings=dir.filePath("ui.ini");
        QSettings preferences(settings,QSettings::IniFormat); preferences.setValue("conversion/autoClose",false); preferences.sync();
        ConversionWindow window(helper,dir.filePath("complete video.mkv"),{},settings); window.show();
        const auto button=window.findChild<QPushButton *>("conversionButton");
        const auto progress=window.findChild<QProgressBar *>("conversionProgress");
        QTRY_COMPARE(progress->value(),42);
        QCOMPARE(button->text(),QString("Cancel"));
        QCOMPARE(window.windowTitle(),QString("Convert only"));
        QCOMPARE(window.findChild<QLabel *>("conversionSourceHeading")->text(),QString("Source"));
        QCOMPARE(window.findChild<QLabel *>("conversionTargetHeading")->text(),QString("Target"));
        for (const auto *name : {"conversionSource","conversionTarget"}) {
            auto *heading=window.findChild<QLabel *>(QString(name)+"Heading");
            QVERIFY(heading->font().bold()); QVERIFY(heading->alignment() & Qt::AlignLeft);
            for (const auto *suffix : {"Container","Video","Resolution","Audio"}) {
                QVERIFY(window.findChild<QLabel *>(QString(name)+suffix+"Label")->font().bold());
                QVERIFY(!window.findChild<QLabel *>(QString(name)+suffix)->font().bold());
            }
        }
        QVERIFY(window.findChild<QLabel *>("selectedVideoHeading")->font().bold());
        QVERIFY(!window.findChild<QLabel *>("selectedVideoFilename")->font().bold());
        QCOMPARE(window.findChild<QLabel *>("selectedVideoHeading")->text(),QString("Selected video"));
        QCOMPARE(window.findChild<QLabel *>("selectedVideoFilename")->text(),QString("complete video.mkv"));
        QVERIFY(window.findChild<QLabel *>("conversionSourceVideo")->text().contains("HEVC"));
        QVERIFY(window.findChild<QLabel *>("conversionTargetVideo")->text().contains("H.264"));
        QVERIFY(window.findChild<QLabel *>("conversionTargetAudio")->text().contains("2 channels"));
        screenshot(window,"conversion-progress");
        QTRY_COMPARE(button->text(),QString("Close"));
        QCOMPARE(progress->value(),100);
        QVERIFY(window.findChild<QLabel *>("conversionOutput")->text().contains("test.yeet-prepared.mp4"));
        QVERIFY(window.findChild<QLabel *>("conversionTargetAudio")->text().contains("AAC"));
        QVERIFY(window.isVisible()); screenshot(window,"conversion-complete");
        const auto countdown=window.findChild<QLabel *>("conversionCountdown");
        QCOMPARE(countdown->text(),QString("Auto-close disabled")); QVERIFY(!countdown->wordWrap());
        QTest::qWait(5500); QVERIFY(window.isVisible());
        QTest::mouseClick(button,Qt::LeftButton); QTRY_VERIFY(!window.isVisible());
        for (const auto &call : calls(helper+".log")) {
            QVERIFY(call["method"]!="discover"); QVERIFY(call["method"]!="start"); QVERIFY(call["method"]!="inspect");
        }
    }
    void conversion_auto_closes_by_default() {
        QTemporaryDir dir; const auto helper=dir.filePath("backend.py");
        QVERIFY(QFile::copy(QStringLiteral(YEET_TEST_BACKEND),helper));
        QVERIFY(QFile::setPermissions(helper,QFile::ReadOwner|QFile::WriteOwner|QFile::ExeOwner));
        ConversionWindow window(helper,dir.filePath("complete.mkv"),{},dir.filePath("default-ui.ini")); window.show();
        QTRY_COMPARE(window.findChild<QPushButton *>("conversionButton")->text(),QString("Close"));
        auto *countdown=window.findChild<QLabel *>("conversionCountdown");
        QCOMPARE(countdown->text(),QString("Closing in 5 seconds…"));
        QVERIFY(!countdown->wordWrap());
        QVERIFY(window.findChild<QLabel *>("conversionOutputLabel")->font().bold());
        screenshot(window,"conversion-auto-close");
        QTest::qWait(1100); QVERIFY(window.isVisible());
        QVERIFY(countdown->text().contains("4 seconds"));
        QTRY_COMPARE_WITH_TIMEOUT(countdown->text(),QString("Closing in 1 second…"),4000);
        QVERIFY(countdown->width()>=countdown->fontMetrics().horizontalAdvance(countdown->text()));
        QTRY_VERIFY_WITH_TIMEOUT(!window.isVisible(),2500);
    }
    void conversion_reuse_keeps_the_same_format_rows() {
        QTemporaryDir dir; const auto helper=dir.filePath("backend.py");
        QVERIFY(QFile::copy(QStringLiteral(YEET_TEST_BACKEND),helper));
        QVERIFY(QFile::setPermissions(helper,QFile::ReadOwner|QFile::WriteOwner|QFile::ExeOwner));
        ConversionWindow window(helper,dir.filePath("reuse video.mkv"),{},dir.filePath("ui.ini")); window.show();
        QTRY_COMPARE(window.findChild<QProgressBar *>("conversionProgress")->value(),42);
        QStringList before;
        for (const auto *name : {"conversionTargetContainer","conversionTargetVideo","conversionTargetResolution","conversionTargetAudio"})
            before << window.findChild<QLabel *>(name)->text();
        screenshot(window,"conversion-reuse-checking");
        QTRY_COMPARE(window.findChild<QPushButton *>("conversionButton")->text(),QString("Close"));
        QStringList after;
        for (const auto *name : {"conversionTargetContainer","conversionTargetVideo","conversionTargetResolution","conversionTargetAudio"})
            after << window.findChild<QLabel *>(name)->text();
        QCOMPARE(before,after);
        QCOMPARE(after[0],QString("MP4")); QVERIFY(after[1].contains("HEVC")); QVERIFY(after[3].contains("6 channels"));
        QCOMPARE(window.findChild<QLabel *>("conversionStatus")->text(),QString("Existing conversion reused"));
        screenshot(window,"conversion-reused");
        window.resize(500,480); QTest::qWait(50);
        auto *countdown=window.findChild<QLabel *>("conversionCountdown");
        QVERIFY(!countdown->wordWrap());
        QVERIFY(countdown->width()>=countdown->fontMetrics().horizontalAdvance(countdown->text()));
        window.close(); QTRY_VERIFY(!window.isVisible());
    }
    void conversion_wrapped_details_keep_enough_height() {
        QTemporaryDir dir; const auto helper=dir.filePath("backend.py");
        QVERIFY(QFile::copy(QStringLiteral(YEET_TEST_BACKEND),helper));
        QVERIFY(QFile::setPermissions(helper,QFile::ReadOwner|QFile::WriteOwner|QFile::ExeOwner));
        const auto settings=dir.filePath("ui.ini");
        QSettings preferences(settings,QSettings::IniFormat); preferences.setValue("conversion/autoClose",false); preferences.sync();
        ConversionWindow window(helper,dir.filePath("wrapped reuse Example.Series.S01E01.1080p.10bit.WEBRip.6CH.x265.HEVC.mkv"),{},settings);
        window.show();
        QTRY_COMPARE(window.findChild<QPushButton *>("conversionButton")->text(),QString("Close"));
        for (const auto width : {740, 560}) {
            window.resize(width,480); QTest::qWait(100);
            screenshot(window,QString("conversion-wrapped-%1").arg(width));
            for (auto *field : window.findChildren<QLabel *>()) {
                if (!field->isVisible() || field->text().isEmpty()) continue;
                const auto required=field->hasHeightForWidth() ? field->heightForWidth(field->width()) : field->minimumSizeHint().height();
                QVERIFY2(field->height()>=required,qPrintable(QString("%1: height %2, needs %3 at width %4").arg(field->objectName()).arg(field->height()).arg(required).arg(field->width())));
            }
        }
        window.close(); QTRY_VERIFY(!window.isVisible());
    }
    void conversion_cancel_failure_and_window_close() {
        QTemporaryDir dir; const auto helper=dir.filePath("backend.py");
        QVERIFY(QFile::copy(QStringLiteral(YEET_TEST_BACKEND),helper));
        QVERIFY(QFile::setPermissions(helper,QFile::ReadOwner|QFile::WriteOwner|QFile::ExeOwner));
        ConversionWindow cancelled(helper,dir.filePath("long video.mkv"),{},dir.filePath("ui.ini")); cancelled.show();
        auto *button=cancelled.findChild<QPushButton *>("conversionButton");
        QTRY_COMPARE(cancelled.findChild<QProgressBar *>("conversionProgress")->value(),42);
        QTest::mouseClick(button,Qt::LeftButton);
        QTRY_COMPARE(button->text(),QString("Close"));
        QVERIFY(cancelled.findChild<QLabel *>("conversionStatus")->text().contains("cleanup completed"));
        QVERIFY(cancelled.findChild<QLabel *>("conversionCountdown")->text().isEmpty());
        QTest::qWait(100); QCOMPARE(button->text(),QString("Close")); // Ignore stale progress.
        QTest::mouseClick(button,Qt::LeftButton); QTRY_VERIFY(!cancelled.isVisible());
        ConversionWindow failed(helper,dir.filePath("fail.mkv"),{},dir.filePath("ui.ini")); failed.show();
        QTRY_COMPARE(failed.findChild<QPushButton *>("conversionButton")->text(),QString("Close"));
        QVERIFY(failed.findChild<QLabel *>("conversionStatus")->text().contains("Test conversion failure"));
        QVERIFY(failed.findChild<QLabel *>("conversionCountdown")->text().isEmpty());
        screenshot(failed,"conversion-failed"); failed.close(); QTRY_VERIFY(!failed.isVisible());
        ConversionWindow active(helper,dir.filePath("long.mkv"),{},dir.filePath("ui.ini")); active.show();
        QTRY_COMPARE(active.findChild<QProgressBar *>("conversionProgress")->value(),42);
        active.close(); QTRY_VERIFY(!active.isVisible());
        ConversionWindow early(helper,dir.filePath("long.mkv"),{},dir.filePath("ui.ini")); early.show();
        QTest::mouseClick(early.findChild<QPushButton *>("conversionButton"),Qt::LeftButton);
        QTRY_COMPARE(early.findChild<QPushButton *>("conversionButton")->text(),QString("Close"));
        early.close(); QTRY_VERIFY(!early.isVisible());
    }
    void explicit_choices_preparation_controls_and_stop() {
        QTemporaryDir dir;
        const auto helper=dir.filePath("backend.py");
        QVERIFY(QFile::copy(QStringLiteral(YEET_TEST_BACKEND),helper));
        QVERIFY(QFile::setPermissions(helper,QFile::ReadOwner|QFile::WriteOwner|QFile::ExeOwner));
        const auto settings=dir.filePath("ui.ini");
        Window window(helper,dir.filePath("test video.mkv"),true,{},settings); window.show();
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
        QCOMPARE(devices->currentIndex(),0); QCOMPARE(subtitles->currentIndex(),1);
        QVERIFY(!yeet->isEnabled()); QVERIFY(resume->isVisible()); QVERIFY(starts(log).isEmpty());
        auto *selected=window.findChild<QGroupBox *>("selectedVideoPanel");
        QVERIFY(selected);
        QCOMPARE(window.findChild<QLabel *>("selectedVideoHeading")->text(),QString("Selected video"));
        QVERIFY(window.findChild<QLabel *>("selectedVideoHeading")->font().bold());
        QCOMPARE(window.findChild<QLabel *>("selectedVideoFilename")->text(),QString("test video.mkv"));
        QVERIFY(!selected->isAncestorOf(window.findChild<QPushButton *>("openVideo")));
        screenshot(window,"selection");
        devices->setCurrentIndex(2); subtitles->setCurrentIndex(2); QVERIFY(yeet->isEnabled());
        QTest::mouseClick(resume,Qt::LeftButton);
        QTRY_COMPARE(progress->value(),42); QCOMPARE(pages->currentIndex(),1);
        QVERIFY(!devices->isEnabled()); QVERIFY(!subtitles->isEnabled()); screenshot(window,"preparing");
        QTRY_COMPARE(starts(log).size(),1);
        QCOMPARE(starts(log).last()["position"].toDouble(),25.0);
        QCOMPARE(starts(log).last()["subtitles"].toObject()["index"].toInt(),3);
        QTRY_COMPARE(pages->currentIndex(),2); screenshot(window,"playing");
        window.activateWindow(); window.findChild<QPushButton *>("stop")->setFocus();
        QTest::keyClick(window.findChild<QPushButton *>("stop"),Qt::Key_Space);
        QTRY_COMPARE(pause->text(),QString("Play"));
        QCOMPARE(pages->currentIndex(),2);
        seek->setValue(50000); QVERIFY(QMetaObject::invokeMethod(seek,"sliderReleased",Qt::DirectConnection));
        QTest::keyClick(seek,Qt::Key_Space); QTRY_COMPARE(pause->text(),QString("Pause"));
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
        QCOMPARE(QSettings(settings,QSettings::IniFormat).value("lastDeviceId").toString(),QString("tv"));
        QSignalSpy exited(window.findChild<Backend *>(),&Backend::exited);
        QTest::mouseClick(window.findChild<QPushButton *>("quit"),Qt::LeftButton);
        QTRY_COMPARE_WITH_TIMEOUT(exited.count(),1,5000); QVERIFY(!window.isVisible());
        Window reopened(helper,{},true,{},settings); reopened.show();
        auto *remembered=reopened.findChild<QComboBox *>("devices");
        QTRY_COMPARE(remembered->currentData().toString(),QString("tv"));
        QSignalSpy reopenedExit(reopened.findChild<Backend *>(),&Backend::exited);
        reopened.close(); QTRY_COMPARE_WITH_TIMEOUT(reopenedExit.count(),1,5000);
    }
    void drops_open_one_local_video_without_starting_playback() {
        QTemporaryDir dir;
        const auto helper=dir.filePath("backend.py");
        QVERIFY(QFile::copy(QStringLiteral(YEET_TEST_BACKEND),helper));
        QVERIFY(QFile::setPermissions(helper,QFile::ReadOwner|QFile::WriteOwner|QFile::ExeOwner));
        const auto settings=dir.filePath("ui.ini");
        QSettings saved(settings,QSettings::IniFormat); saved.setValue("lastDeviceId","speaker"); saved.sync();
        Window window(helper,{},true,{},settings); window.show();
        auto *devices=window.findChild<QComboBox *>("devices");
        auto *subtitles=window.findChild<QComboBox *>("subtitles");
        QTRY_COMPARE(devices->count(),3);
        QCOMPARE(devices->currentIndex(),0); // Never restore an audio-only target.
        const auto file=dir.filePath("dropped video #1.mkv");
        QFile video(file); QVERIFY(video.open(QIODevice::WriteOnly)); video.write("fixture"); video.close();
        QMimeData mime;
        for (const auto &urls : {QList<QUrl>{QUrl("https://example.org/video.mp4")},
                               QList<QUrl>{QUrl::fromLocalFile(dir.path())},
                               QList<QUrl>{QUrl::fromLocalFile(file),QUrl::fromLocalFile(file)}}) {
            mime.setUrls(urls);
            QDragEnterEvent enter(QPoint(30,30),Qt::CopyAction,&mime,Qt::LeftButton,Qt::NoModifier);
            QApplication::sendEvent(&window,&enter); QVERIFY(!enter.isAccepted());
        }
        mime.setUrls({QUrl::fromLocalFile(file)});
        QDragEnterEvent enter(QPoint(30,30),Qt::CopyAction|Qt::MoveAction,&mime,Qt::LeftButton,Qt::NoModifier);
        QApplication::sendEvent(&window,&enter); QVERIFY(enter.isAccepted());
        QDropEvent drop(QPointF(30,30),Qt::CopyAction|Qt::MoveAction,&mime,Qt::LeftButton,Qt::NoModifier);
        QApplication::sendEvent(&window,&drop); QVERIFY(drop.isAccepted()); QCOMPARE(drop.dropAction(),Qt::CopyAction);
        QTRY_VERIFY(subtitles->isEnabled()); QCOMPARE(subtitles->currentIndex(),1);
        QVERIFY(starts(helper+".log").isEmpty());
        devices->setCurrentIndex(2);
        QTest::mouseClick(window.findChild<QPushButton *>("yeet"),Qt::LeftButton);
        QTRY_COMPARE(starts(helper+".log").size(),1);
        QCOMPARE(starts(helper+".log").last()["file"].toString(),file);
        QDragEnterEvent duringPlayback(QPoint(30,30),Qt::CopyAction,&mime,Qt::LeftButton,Qt::NoModifier);
        QApplication::sendEvent(&window,&duringPlayback); QVERIFY(!duringPlayback.isAccepted());
        QSignalSpy exited(window.findChild<Backend *>(),&Backend::exited);
        window.activateWindow(); QTest::keyClick(&window,Qt::Key_Q,Qt::ControlModifier);
        QTRY_COMPARE_WITH_TIMEOUT(exited.count(),1,5000);
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
        Window window(wrapper,file,false,{},dir.filePath("ui.ini")); window.show();
        QTRY_VERIFY_WITH_TIMEOUT(window.findChild<QComboBox *>("subtitles")->isEnabled(),5000);
        QVERIFY(!window.findChild<QPushButton *>("yeet")->isEnabled());
        QCOMPARE(window.findChild<QStackedWidget *>("pages")->currentIndex(),0);
        QSignalSpy exited(window.findChild<Backend *>(),&Backend::exited);
        window.close(); QTRY_COMPARE_WITH_TIMEOUT(exited.count(),1,5000);
    }
};
QTEST_MAIN(WindowTest)
#include "window-test.moc"
