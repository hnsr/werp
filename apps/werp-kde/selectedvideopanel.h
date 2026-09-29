#pragma once
#include <KLocalizedString>
#include <QFileInfo>
#include <QGroupBox>
#include <QLabel>
#include <QVBoxLayout>

// Shared spacing keeps the player and conversion window visually consistent.
inline void setWindowSpacing(QVBoxLayout *layout) {
    layout->setContentsMargins(24,20,24,20);
    layout->setSpacing(16);
}

class SelectedVideoPanel final : public QGroupBox {
public:
    explicit SelectedVideoPanel(QWidget *parent, const QString &file = {}) : QGroupBox(parent) {
        setObjectName("selectedVideoPanel");
        auto *layout=new QVBoxLayout(this);
        layout->setContentsMargins(16,16,16,16); layout->setSpacing(8);
        auto *heading=new QLabel(i18n("Selected video"),this);
        heading->setObjectName("selectedVideoHeading");
        heading->setTextFormat(Qt::PlainText); heading->setAlignment(Qt::AlignLeft|Qt::AlignTop);
        auto headingFont=heading->font(); headingFont.setBold(true); heading->setFont(headingFont);
        layout->addWidget(heading);
        m_filename=new QLabel(this); m_filename->setObjectName("selectedVideoFilename");
        m_filename->setTextFormat(Qt::PlainText); m_filename->setWordWrap(true);
        m_filename->setAlignment(Qt::AlignLeft|Qt::AlignTop);
        m_filename->setTextInteractionFlags(Qt::TextSelectableByMouse);
        layout->addWidget(m_filename);
        m_subtitle=new QLabel(this); m_subtitle->setObjectName("activeSubtitle");
        m_subtitle->setTextFormat(Qt::PlainText); m_subtitle->setWordWrap(true);
        m_subtitle->setAlignment(Qt::AlignLeft|Qt::AlignTop);
        m_subtitle->setTextInteractionFlags(Qt::TextSelectableByMouse);
        layout->addWidget(m_subtitle);
        setActiveSubtitle({});
        setFile(file);
    }
    void setFile(const QString &file) {
        m_filename->setText(file.isEmpty() ? i18n("Choose a video to get started") : QFileInfo(file).fileName());
        m_filename->setToolTip(file.isEmpty() ? QString() : QFileInfo(file).absoluteFilePath());
    }
    void setActiveSubtitle(const QString &subtitle) {
        m_subtitle->setText(subtitle.isEmpty() ? QString() : i18n("Subtitles: %1",subtitle));
        m_subtitle->setToolTip(m_subtitle->text());
        m_subtitle->setVisible(!subtitle.isEmpty());
    }
private:
    QLabel *m_filename;
    QLabel *m_subtitle;
};
