#include "ZoneMargin.hpp"
#include <QFont>
#include <QFontDatabase>
#include <QScrollArea>
#include <QFrame>

namespace kala {

ZoneMargin::ZoneMargin(QWidget* parent) : QWidget(parent) {
    setupUi();
    applyStyle();
    setFixedWidth(260);
    setMinimumWidth(240);
    setMaximumWidth(280);
}

void ZoneMargin::setupUi() {
    auto* scrollArea = new QScrollArea(this);
    scrollArea->setWidgetResizable(true);
    scrollArea->setFrameShape(QFrame::NoFrame);
    scrollArea->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    scrollArea->setStyleSheet("QScrollArea { background: transparent; border: none; }");

    auto* contentWidget = new QWidget();
    contentWidget->setObjectName("marginContent");
    auto* layout = new QVBoxLayout(contentWidget);
    layout->setContentsMargins(8, 8, 8, 8);
    layout->setSpacing(12);

    createSessionGroup(layout);
    createFreeCadGroup(layout);
    createExportGroup(layout);
    createProcedureGroup(layout);

    layout->addStretch();

    scrollArea->setWidget(contentWidget);

    auto* mainLayout = new QVBoxLayout(this);
    mainLayout->setContentsMargins(0, 0, 0, 0);
    mainLayout->addWidget(scrollArea);
}

void ZoneMargin::createSessionGroup(QVBoxLayout* layout) {
    auto* group = new QGroupBox("SESSION", this);
    group->setObjectName("marginGroup");
    auto* gl = new QVBoxLayout(group);
    gl->setContentsMargins(8, 14, 8, 8);
    gl->setSpacing(4);

    QFont monoFont = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    monoFont.setPointSize(10);

    m_sessionStatus = new QLabel("status: idle", group);
    m_sessionStatus->setFont(monoFont);
    m_sessionStatus->setObjectName("marginValue");
    gl->addWidget(m_sessionStatus);

    m_backendStatus = new QLabel("backend: disconnected", group);
    m_backendStatus->setFont(monoFont);
    m_backendStatus->setObjectName("marginValue");
    gl->addWidget(m_backendStatus);

    layout->addWidget(group);
}

void ZoneMargin::createFreeCadGroup(QVBoxLayout* layout) {
    auto* group = new QGroupBox("FREECAD / TOOLS", this);
    group->setObjectName("marginGroup");
    auto* gl = new QVBoxLayout(group);
    gl->setContentsMargins(8, 14, 8, 8);
    gl->setSpacing(4);

    QFont monoFont = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    monoFont.setPointSize(10);

    m_freeCadStatus = new QLabel("FreeCAD: \u25cb disconnected", group);
    m_freeCadStatus->setFont(monoFont);
    m_freeCadStatus->setObjectName("marginValue");
    gl->addWidget(m_freeCadStatus);

    m_toolsStatus = new QLabel("tools: \u2014", group);
    m_toolsStatus->setFont(monoFont);
    m_toolsStatus->setObjectName("marginValue");
    gl->addWidget(m_toolsStatus);

    m_openFreeCadBtn = new QPushButton("Show in FreeCAD", group);
    m_openFreeCadBtn->setObjectName("marginButton");
    m_openFreeCadBtn->setCursor(Qt::PointingHandCursor);
    connect(m_openFreeCadBtn, &QPushButton::clicked, this, &ZoneMargin::openInFreeCadRequested);
    gl->addWidget(m_openFreeCadBtn);

    layout->addWidget(group);
}

void ZoneMargin::createExportGroup(QVBoxLayout* layout) {
    auto* group = new QGroupBox("EXPORT", this);
    group->setObjectName("marginGroup");
    auto* gl = new QVBoxLayout(group);
    gl->setContentsMargins(8, 14, 8, 8);
    gl->setSpacing(6);

    QFont monoFont = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    monoFont.setPointSize(10);

    m_exportPath = new QLabel("path: \u2014", group);
    m_exportPath->setFont(monoFont);
    m_exportPath->setObjectName("marginValue");
    m_exportPath->setWordWrap(true);
    gl->addWidget(m_exportPath);

    auto* btnLayout = new QHBoxLayout();
    btnLayout->setSpacing(6);

    m_exportStepBtn = new QPushButton("STEP", group);
    m_exportStepBtn->setObjectName("marginButtonSmall");
    m_exportStepBtn->setCursor(Qt::PointingHandCursor);
    m_exportStepBtn->setEnabled(false);
    connect(m_exportStepBtn, &QPushButton::clicked, this, &ZoneMargin::exportStepRequested);
    btnLayout->addWidget(m_exportStepBtn);

    m_exportStlBtn = new QPushButton("STL", group);
    m_exportStlBtn->setObjectName("marginButtonSmall");
    m_exportStlBtn->setCursor(Qt::PointingHandCursor);
    m_exportStlBtn->setEnabled(false);
    connect(m_exportStlBtn, &QPushButton::clicked, this, &ZoneMargin::exportStlRequested);
    btnLayout->addWidget(m_exportStlBtn);

    gl->addLayout(btnLayout);
    layout->addWidget(group);
}

void ZoneMargin::createProcedureGroup(QVBoxLayout* layout) {
    auto* group = new QGroupBox("PROCEDURE", this);
    group->setObjectName("marginGroup");
    auto* gl = new QVBoxLayout(group);
    gl->setContentsMargins(8, 14, 8, 8);
    gl->setSpacing(6);

    QFont monoFont = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    monoFont.setPointSize(10);

    m_procedureCombo = new QComboBox(group);
    m_procedureCombo->setObjectName("marginCombo");
    m_procedureCombo->addItem("L-bracket", "l_bracket");
    m_procedureCombo->addItem("Custom...", "custom");
    connect(m_procedureCombo, QOverload<int>::of(&QComboBox::currentIndexChanged),
            this, [this](int index) {
                QString id = m_procedureCombo->itemData(index).toString();
                if (id != "custom") {
                    emit procedureChanged(id);
                }
            });
    gl->addWidget(m_procedureCombo);

    m_stepList = new QListWidget(group);
    m_stepList->setObjectName("stepList");
    m_stepList->setMaximumHeight(160);
    m_stepList->setFont(monoFont);
    m_stepList->setFrameShape(QFrame::NoFrame);
    gl->addWidget(m_stepList);

    auto* tipLabel = new QLabel("tip: sheet vs tools", group);
    tipLabel->setFont(monoFont);
    tipLabel->setObjectName("tipLabel");
    tipLabel->setWordWrap(true);
    gl->addWidget(tipLabel);

    layout->addWidget(group);
}

void ZoneMargin::applyStyle() {
    setObjectName("margin");
    setStyleSheet(R"(
        #margin {
            background-color: #0B0D10;
            border-left: 1px solid #1E2329;
        }
        #marginContent {
            background: transparent;
        }
        #marginGroup {
            background-color: #12151A;
            border: 1px solid #1E2329;
            border-radius: 0px;
            margin-top: 8px;
            padding-top: 4px;
            color: #A8B0BA;
            font-weight: 600;
            font-size: 10px;
            letter-spacing: 0.5px;
        }
        #marginGroup::title {
            subcontrol-origin: margin;
            left: 8px;
            padding: 0 4px;
            color: #5B9FD4;
        }
        #marginValue {
            color: #A8B0BA;
        }
        #marginButton {
            background-color: #161A20;
            border: 1px solid #1E2329;
            color: #A8B0BA;
            padding: 6px 10px;
            font-size: 10px;
            font-family: monospace;
        }
        #marginButton:hover {
            border-color: #5B9FD4;
            color: #5B9FD4;
        }
        #marginButtonSmall {
            background-color: #161A20;
            border: 1px solid #1E2329;
            color: #A8B0BA;
            padding: 4px 8px;
            font-size: 10px;
            font-family: monospace;
        }
        #marginButtonSmall:hover {
            border-color: #5B9FD4;
            color: #5B9FD4;
        }
        #marginButtonSmall:disabled {
            color: #3A3F44;
            border-color: #1E2329;
        }
        #marginCombo {
            background-color: #161A20;
            border: 1px solid #1E2329;
            color: #A8B0BA;
            padding: 4px 8px;
            font-size: 10px;
            font-family: monospace;
        }
        #marginCombo:hover {
            border-color: #5B9FD4;
        }
        #marginCombo::drop-down {
            border: none;
            width: 20px;
        }
        #stepList {
            background-color: #0B0D10;
            border: 1px solid #1E2329;
            color: #A8B0BA;
            outline: none;
        }
        #stepList::item {
            padding: 3px 6px;
            border-bottom: 1px solid #1E2329;
        }
        #stepList::item:selected {
            background-color: #5B9FD4;
            color: #0B0D10;
        }
        #tipLabel {
            color: #4A525A;
            font-style: italic;
        }
        QScrollBar:vertical {
            background: #0B0D10;
            width: 6px;
            border: none;
        }
        QScrollBar::handle:vertical {
            background: #1E2329;
            min-height: 30px;
            border-radius: 3px;
        }
        QScrollBar::handle:vertical:hover {
            background: #5B9FD4;
        }
    )");
}

void ZoneMargin::updateData(const MarginData& data) {
    m_sessionStatus->setText(QString("status: %1").arg(data.session.status));
    m_backendStatus->setText(QString("backend: %1").arg(data.session.plannerModel.isEmpty() ? "disconnected" : "connected"));
    m_freeCadStatus->setText(QString("FreeCAD: %1 %2")
        .arg(data.session.freeCadConnected ? "\u25cf" : "\u25cb")
        .arg(data.session.freeCadConnected ? "connected" : "disconnected"));
    m_toolsStatus->setText(QString("tools: %1").arg(data.toolsStatus.isEmpty() ? "\u2014" : data.toolsStatus));

    QString exportPath = data.exportInfo.stepPath;
    if (exportPath.isEmpty()) exportPath = data.exportInfo.stlPath;
    if (exportPath.isEmpty()) exportPath = data.exportInfo.assemblyPath;
    m_exportPath->setText(QString("path: %1").arg(exportPath.isEmpty() ? "\u2014" : exportPath));

    m_exportStepBtn->setEnabled(data.exportInfo.hasStep);
    m_exportStlBtn->setEnabled(data.exportInfo.hasStl);

    m_stepList->clear();
    for (int i = 0; i < data.procedureSteps.size(); ++i) {
        QString step = data.procedureSteps[i];
        if (i == data.currentStepIndex) {
            step = "\u25b6 " + step;
        } else if (i < data.currentStepIndex) {
            step = "\u2713 " + step;
        } else {
            step = "  " + step;
        }
        m_stepList->addItem(step);
    }
    if (data.currentStepIndex >= 0 && data.currentStepIndex < m_stepList->count()) {
        m_stepList->setCurrentRow(data.currentStepIndex);
        m_stepList->scrollToItem(m_stepList->item(data.currentStepIndex));
    }
}

} // namespace kala