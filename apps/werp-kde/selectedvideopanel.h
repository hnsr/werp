#pragma once
#include <KLocalizedString>
#include <QFileInfo>
#include <QGroupBox>
#include <QGridLayout>
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
        m_heading=new QLabel(i18n("Selected video"),this);
        m_heading->setObjectName("selectedVideoHeading");
        m_heading->setTextFormat(Qt::PlainText); m_heading->setAlignment(Qt::AlignLeft|Qt::AlignTop);
        auto headingFont=m_heading->font(); headingFont.setBold(true); m_heading->setFont(headingFont);
        layout->addWidget(m_heading);
        m_filename=new QLabel(this); m_filename->setObjectName("selectedVideoFilename");
        m_filename->setTextFormat(Qt::PlainText); m_filename->setWordWrap(true);
        m_filename->setAlignment(Qt::AlignLeft|Qt::AlignTop);
        m_filename->setTextInteractionFlags(Qt::TextSelectableByMouse);
        layout->addWidget(m_filename);
        m_details=new QWidget(this);
        auto *detailsLayout=new QGridLayout(m_details);
        detailsLayout->setContentsMargins(0,8,0,0);
        detailsLayout->setHorizontalSpacing(4);
        detailsLayout->setVerticalSpacing(4);
        auto *subtitleLabel=new QLabel(i18n("Subtitles:"),m_details);
        subtitleLabel->setObjectName("activeSubtitleLabel");
        auto labelFont=subtitleLabel->font(); labelFont.setBold(true);
        subtitleLabel->setFont(labelFont);
        subtitleLabel->setAlignment(Qt::AlignLeft|Qt::AlignTop);
        detailsLayout->addWidget(subtitleLabel,0,0);
        m_subtitle=new QLabel(m_details); m_subtitle->setObjectName("activeSubtitle");
        m_subtitle->setTextFormat(Qt::PlainText); m_subtitle->setWordWrap(true);
        m_subtitle->setAlignment(Qt::AlignLeft|Qt::AlignTop);
        m_subtitle->setTextInteractionFlags(Qt::TextSelectableByMouse);
        detailsLayout->addWidget(m_subtitle,0,1);
        auto *receiverLabel=new QLabel(i18n("Receiver:"),m_details);
        receiverLabel->setObjectName("activeReceiverLabel");
        receiverLabel->setFont(labelFont);
        receiverLabel->setAlignment(Qt::AlignLeft|Qt::AlignTop);
        detailsLayout->addWidget(receiverLabel,1,0);
        m_receiver=new QLabel(m_details); m_receiver->setObjectName("activeReceiver");
        m_receiver->setTextFormat(Qt::PlainText); m_receiver->setWordWrap(true);
        m_receiver->setAlignment(Qt::AlignLeft|Qt::AlignTop);
        m_receiver->setTextInteractionFlags(Qt::TextSelectableByMouse);
        detailsLayout->addWidget(m_receiver,1,1);
        detailsLayout->setColumnStretch(1,1);
        layout->addWidget(m_details);
        setPlaybackDetails({},{});
        setFile(file);
    }
    void setFile(const QString &file) {
        m_filename->setText(file.isEmpty() ? i18n("Choose a video to get started") : QFileInfo(file).fileName());
        m_filename->setToolTip(file.isEmpty() ? QString() : QFileInfo(file).absoluteFilePath());
    }
    void setPlaybackDetails(const QString &subtitle, const QString &receiver) {
        m_heading->setText(receiver.isEmpty() ? i18n("Selected video") : i18n("Now casting"));
        m_subtitle->setText(subtitle);
        m_receiver->setText(receiver);
        m_details->setVisible(!receiver.isEmpty());
    }
private:
    QLabel *m_heading, *m_filename;
    QWidget *m_details;
    QLabel *m_subtitle, *m_receiver;
};
