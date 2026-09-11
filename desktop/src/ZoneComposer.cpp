#include "ZoneComposer.hpp"
#include <QFont>
#include <QFontDatabase>

namespace kala {

ZoneComposer::ZoneComposer(QWidget* parent) : QWidget(parent) {
    setupUi();
    applyStyle();
    setFixedHeight(140);
}

void ZoneComposer::setupUi() {
    auto* layout = new QVBoxLayout(this);
    layout->setContentsMargins(12, 8, 12, 8);
    layout->setSpacing(8);

    auto* topRow = new QHBoxLayout();
    topRow->setSpacing(8);

    auto* starterLabel = new QLabel("STARTER", this);
    starterLabel->setObjectName("composerLabel");
    QFont labelFont = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    labelFont.setPointSize(10);
    labelFont.setWeight(QFont::DemiBold);
    labelFont.setLetterSpacing(QFont::AbsoluteSpacing, 0.5);
    starterLabel->setFont(labelFont);
    topRow->addWidget(starterLabel);

    m_starterCombo = new QComboBox(this);
    m_starterCombo->setObjectName("starterCombo");
    m_starterCombo->addItem("L-bracket (mm brief)", "l_bracket");
    m_starterCombo->addItem("Custom procedure", "custom");
    {
        QFont font = QFontDatabase::systemFont(QFontDatabase::FixedFont);
        font.setPointSize(11);
        m_starterCombo->setFont(font);
    }
    connect(m_starterCombo, QOverload<int>::of(&QComboBox::currentIndexChanged),
            this, [this](int index) {
                emit starterChanged(m_starterCombo->itemData(index).toString());
            });
    topRow->addWidget(m_starterCombo, 1);

    m_providerLabel = new QLabel("no API key", this);
    m_providerLabel->setObjectName("providerLabel");
    m_providerLabel->setFont(labelFont);
    topRow->addWidget(m_providerLabel);

    layout->addLayout(topRow);

    m_briefEdit = new QTextEdit(this);
    m_briefEdit->setObjectName("briefEdit");
    m_briefEdit->setPlaceholderText("Describe the part in mm... e.g., \"L-bracket, 50x30x5, 6mm holes, 10mm fillets\"");
    {
        QFont font = QFontDatabase::systemFont(QFontDatabase::FixedFont);
        font.setPointSize(12);
        m_briefEdit->setFont(font);
    }
    m_briefEdit->setLineWrapMode(QTextEdit::WidgetWidth);
    layout->addWidget(m_briefEdit, 1);

    auto* bottomRow = new QHBoxLayout();
    bottomRow->setSpacing(8);

    bottomRow->addStretch();

    m_runButton = new QPushButton("Run", this);
    m_runButton->setObjectName("runButton");
    m_runButton->setFixedWidth(100);
    m_runButton->setFixedHeight(36);
    m_runButton->setCursor(Qt::PointingHandCursor);
    m_runButton->setFont(labelFont);
    connect(m_runButton, &QPushButton::clicked, this, [this]() {
        emit runRequested(m_briefEdit->toPlainText().trimmed(),
                         m_starterCombo->currentData().toString());
    });
    bottomRow->addWidget(m_runButton);

    layout->addLayout(bottomRow);
}

void ZoneComposer::applyStyle() {
    setObjectName("composer");
    setStyleSheet(R"(
        #composer {
            background-color: #0B0D10;
            border-top: 1px solid #1E2329;
        }
        #composerLabel {
            color: #5B9FD4;
        }
        #starterCombo {
            background-color: #161A20;
            border: 1px solid #1E2329;
            color: #A8B0BA;
            padding: 6px 10px;
            font-family: monospace;
        }
        #starterCombo:hover {
            border-color: #5B9FD4;
        }
        #starterCombo::drop-down {
            border: none;
            width: 24px;
        }
        #providerLabel {
            color: #E06C75;
        }
        #providerLabel[hasKey="true"] {
            color: #7FB069;
        }
        #briefEdit {
            background-color: #12151A;
            border: 1px solid #1E2329;
            color: #E8EDF2;
            padding: 10px;
            font-family: monospace;
            selection-background-color: #5B9FD4;
        }
        #briefEdit:focus {
            border-color: #5B9FD4;
        }
        #briefEdit::placeholder {
            color: #4A525A;
        }
        #runButton {
            background-color: #5B9FD4;
            border: none;
            color: #0B0D10;
            font-weight: 600;
            letter-spacing: 0.5px;
        }
        #runButton:hover {
            background-color: #7AB8E8;
        }
        #runButton:pressed {
            background-color: #4A8BC4;
        }
        #runButton:disabled {
            background-color: #2A2F35;
            color: #4A525A;
        }
    )");
}

void ZoneComposer::setState(const ComposerState& state) {
    m_briefEdit->setPlainText(state.briefText);
    m_isRunning = state.isRunning;
    m_runButton->setEnabled(!state.isRunning && !state.briefText.trimmed().isEmpty());
    m_runButton->setText(state.isRunning ? "Working..." : "Run");

    int idx = m_starterCombo->findData(state.currentStarter);
    if (idx >= 0) {
        m_starterCombo->setCurrentIndex(idx);
    }

    m_providerLabel->setText(state.hasProviderKey ? "API key configured" : "no API key");
    m_providerLabel->setProperty("hasKey", state.hasProviderKey);
    m_providerLabel->style()->unpolish(m_providerLabel);
    m_providerLabel->style()->polish(m_providerLabel);
}

QString ZoneComposer::getBriefText() const {
    return m_briefEdit->toPlainText().trimmed();
}

} // namespace kala