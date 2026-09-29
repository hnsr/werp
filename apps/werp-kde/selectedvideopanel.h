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
        setFile(file);
    }
    void setFile(const QString &file) {
        m_filename->setText(file.isEmpty() ? i18n("Choose a video to get started") : QFileInfo(file).fileName());
        m_filename->setToolTip(file.isEmpty() ? QString() : QFileInfo(file).absoluteFilePath());
    }
private:
    QLabel *m_filename;
};
