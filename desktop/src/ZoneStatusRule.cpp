#include "ZoneStatusRule.hpp"
#include <QFont>
#include <QFontDatabase>

namespace kala {

ZoneStatusRule::ZoneStatusRule(QWidget* parent) : QWidget(parent) {
    setupUi();
    applyStyle();
    setFixedHeight(28);
}

void ZoneStatusRule::setupUi() {
    auto* layout = new QHBoxLayout(this);
    layout->setContentsMargins(12, 0, 12, 0);
    layout->setSpacing(20);

    QFont monoFont = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    monoFont.setPointSize(10);
    monoFont.setWeight(QFont::Medium);

    m_unitsLabel = new QLabel("mm", this);
    m_unitsLabel->setFont(monoFont);
    m_unitsLabel->setObjectName("statusItem");
    layout->addWidget(m_unitsLabel);

    auto* sep1 = new QLabel("\u2502", this);
    sep1->setObjectName("statusSep");
    layout->addWidget(sep1);

    m_stepLabel = new QLabel("STEP: none", this);
    m_stepLabel->setFont(monoFont);
    m_stepLabel->setObjectName("statusItem");
    layout->addWidget(m_stepLabel);

    auto* sep2 = new QLabel("\u2502", this);
    sep2->setObjectName("statusSep");
    layout->addWidget(sep2);

    m_playbookLabel = new QLabel("playbook: gated", this);
    m_playbookLabel->setFont(monoFont);
    m_playbookLabel->setObjectName("statusItem");
    layout->addWidget(m_playbookLabel);

    auto* sep3 = new QLabel("\u2502", this);
    sep3->setObjectName("statusSep");
    layout->addWidget(sep3);

    m_backendLabel = new QLabel("backend: disconnected", this);
    m_backendLabel->setFont(monoFont);
    m_backendLabel->setObjectName("statusItem");
    layout->addWidget(m_backendLabel);

    auto* sep4 = new QLabel("\u2502", this);
    sep4->setObjectName("statusSep");
    layout->addWidget(sep4);

    m_partsLabel = new QLabel("parts: off", this);
    m_partsLabel->setFont(monoFont);
    m_partsLabel->setObjectName("statusItem");
    layout->addWidget(m_partsLabel);

    layout->addStretch();
}

void ZoneStatusRule::applyStyle() {
    setObjectName("statusRule");
    setStyleSheet(R"(
        #statusRule {
            background-color: #0B0D10;
            border-top: 1px solid #1E2329;
        }
        #statusItem {
            color: #A8B0BA;
        }
        #statusSep {
            color: #1E2329;
        }
        #unitsLabel {
            color: #5B9FD4;
            font-weight: 600;
        }
    )");
}

void ZoneStatusRule::updateData(const StatusRuleData& data) {
    m_unitsLabel->setText(data.units);
    m_stepLabel->setText(data.stepStatus);
    m_playbookLabel->setText(data.playbookStatus);
    m_backendLabel->setText(data.backendStatus);
    m_partsLabel->setText(data.partsStatus);
}

} // namespace kala