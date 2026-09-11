#include "ZoneTitleBlock.hpp"
#include <QFont>
#include <QFontDatabase>
#include <QStyle>

namespace kala {

ZoneTitleBlock::ZoneTitleBlock(QWidget* parent) : QWidget(parent) {
    setupUi();
    applyStyle();
    setFixedHeight(52);
}

void ZoneTitleBlock::setupUi() {
    auto* layout = new QHBoxLayout(this);
    layout->setContentsMargins(12, 4, 12, 4);
    layout->setSpacing(16);

    QFont monoFont = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    monoFont.setPointSize(11);
    monoFont.setWeight(QFont::Medium);

    QFont labelFont = monoFont;
    labelFont.setWeight(QFont::DemiBold);
    labelFont.setPointSize(10);

    m_appLabel = new QLabel("KALA", this);
    m_appLabel->setFont(labelFont);
    m_appLabel->setObjectName("appLabel");
    layout->addWidget(m_appLabel);

    auto* sep1 = new QLabel("\u2502", this);
    sep1->setObjectName("separator");
    layout->addWidget(sep1);

    m_sessionStatus = new QLabel("session: \u2014", this);
    m_sessionStatus->setFont(monoFont);
    m_sessionStatus->setObjectName("sessionStatus");
    layout->addWidget(m_sessionStatus);

    auto* sep2 = new QLabel("\u2502", this);
    sep2->setObjectName("separator");
    layout->addWidget(sep2);

    m_procedureId = new QLabel("proc: \u2014", this);
    m_procedureId->setFont(monoFont);
    m_procedureId->setObjectName("procedureId");
    layout->addWidget(m_procedureId);

    auto* sep3 = new QLabel("\u2502", this);
    sep3->setObjectName("separator");
    layout->addWidget(sep3);

    m_plannerModel = new QLabel("planner: \u2014", this);
    m_plannerModel->setFont(monoFont);
    m_plannerModel->setObjectName("plannerModel");
    layout->addWidget(m_plannerModel);

    auto* sep4 = new QLabel("\u2502", this);
    sep4->setObjectName("separator");
    layout->addWidget(sep4);

    m_freeCadSync = new QLabel("FreeCAD: \u25cb disconnected", this);
    m_freeCadSync->setFont(monoFont);
    m_freeCadSync->setObjectName("freeCadSync");
    layout->addWidget(m_freeCadSync);

    layout->addStretch();

    m_unitsLabel = new QLabel("mm", this);
    m_unitsLabel->setFont(monoFont);
    m_unitsLabel->setObjectName("unitsLabel");
    layout->addWidget(m_unitsLabel);

    auto* sep5 = new QLabel("\u2502", this);
    sep5->setObjectName("separator");
    layout->addWidget(sep5);

    m_revisionLabel = new QLabel("rev: \u2014", this);
    m_revisionLabel->setFont(monoFont);
    m_revisionLabel->setObjectName("revisionLabel");
    layout->addWidget(m_revisionLabel);
}

void ZoneTitleBlock::applyStyle() {
    setObjectName("titleBlock");
    setStyleSheet(R"(
        #titleBlock {
            background-color: #0B0D10;
            border-bottom: 1px solid #1E2329;
        }
        #appLabel {
            color: #5B9FD4;
            letter-spacing: 0.5px;
        }
        #separator {
            color: #1E2329;
        }
        #sessionStatus,
        #procedureId,
        #plannerModel,
        #freeCadSync,
        #unitsLabel,
        #revisionLabel {
            color: #A8B0BA;
        }
        #freeCadSync[connected="true"] {
            color: #5B9FD4;
        }
        #unitsLabel {
            color: #5B9FD4;
            font-weight: 600;
        }
    )");
}

void ZoneTitleBlock::updateSession(const SessionInfo& info) {
    m_sessionStatus->setText(QString("session: %1").arg(info.sessionId.isEmpty() ? "\u2014" : info.sessionId.left(12)));
    m_procedureId->setText(QString("proc: %1").arg(info.procedureId.isEmpty() ? "\u2014" : info.procedureId));
    m_plannerModel->setText(QString("planner: %1").arg(info.plannerModel.isEmpty() ? "\u2014" : info.plannerModel));
    m_freeCadSync->setText(QString("FreeCAD: %1 %2")
        .arg(info.freeCadConnected ? "\u25cf" : "\u25cb")
        .arg(info.freeCadConnected ? "connected" : "disconnected"));
    m_freeCadSync->setProperty("connected", info.freeCadConnected);
    m_freeCadSync->style()->unpolish(m_freeCadSync);
    m_freeCadSync->style()->polish(m_freeCadSync);
    m_revisionLabel->setText(QString("rev: %1").arg(info.status));
}

} // namespace kala