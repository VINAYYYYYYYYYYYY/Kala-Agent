#include "ZoneFeed.hpp"
#include <QFont>
#include <QFontDatabase>
#include <QDateTime>
#include <QScrollBar>

namespace kala {

ZoneFeed::ZoneFeed(QWidget* parent) : QWidget(parent) {
    setupUi();
    applyStyle();
    setMinimumHeight(280);
    setMaximumHeight(360);
}

void ZoneFeed::setupUi() {
    auto* layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);

    auto* header = new QLabel("TRACE", this);
    header->setObjectName("feedHeader");
    QFont headerFont = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    headerFont.setPointSize(10);
    headerFont.setWeight(QFont::DemiBold);
    headerFont.setLetterSpacing(QFont::AbsoluteSpacing, 0.5);
    header->setFont(headerFont);
    layout->addWidget(header);

    m_list = new QListWidget(this);
    m_list->setObjectName("feedList");
    m_list->setFrameShape(QFrame::NoFrame);
    m_list->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_list->setVerticalScrollMode(QAbstractItemView::ScrollPerPixel);
    {
        QFont font = QFontDatabase::systemFont(QFontDatabase::FixedFont);
        font.setPointSize(11);
        m_list->setFont(font);
    }
    layout->addWidget(m_list, 1);

    m_emptyLabel = new QLabel("No trace yet. Run a procedure to see tool lines.", this);
    m_emptyLabel->setObjectName("emptyLabel");
    m_emptyLabel->setAlignment(Qt::AlignCenter);
    {
        QFont font = QFontDatabase::systemFont(QFontDatabase::FixedFont);
        font.setPointSize(11);
        m_emptyLabel->setFont(font);
    }
    m_emptyLabel->hide();
    layout->addWidget(m_emptyLabel);
}

void ZoneFeed::applyStyle() {
    setObjectName("feed");
    setStyleSheet(R"(
        #feed {
            background-color: #0B0D10;
            border-top: 1px solid #1E2329;
        }
        #feedHeader {
            background-color: #12151A;
            color: #5B9FD4;
            padding: 6px 12px;
            border-bottom: 1px solid #1E2329;
            letter-spacing: 0.5px;
        }
        #feedList {
            background-color: #0B0D10;
            border: none;
            color: #A8B0BA;
            outline: none;
            padding: 4px 8px;
        }
        #feedList::item {
            padding: 2px 4px;
            margin: 1px 0;
            border-left: 2px solid transparent;
        }
        #feedList::item:selected {
            background-color: #161A20;
            border-left-color: #5B9FD4;
        }
        #emptyLabel {
            color: #4A525A;
            padding: 20px;
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

void ZoneFeed::addToolLine(const ToolLine& line) {
    m_emptyLabel->hide();

    QString prefix;
    QColor color = QColor("#A8B0BA");

    switch (line.type) {
        case ToolLine::Type::Info:
            prefix = "  ";
            color = QColor("#6A727A");
            break;
        case ToolLine::Type::ToolCall:
            prefix = "\u25b6 ";
            color = QColor("#5B9FD4");
            break;
        case ToolLine::Type::ToolResult:
            prefix = "\u2713 ";
            color = QColor("#7FB069");
            break;
        case ToolLine::Type::Error:
            prefix = "\u2717 ";
            color = QColor("#E06C75");
            break;
        case ToolLine::Type::Summary:
            prefix = "\u25aa ";
            color = QColor("#D19A66");
            break;
    }

    QString timestamp = line.timestamp.isEmpty()
        ? QDateTime::currentDateTime().toString("HH:mm:ss.zzz")
        : line.timestamp;

    QString displayText;
    if (!line.toolName.isEmpty()) {
        displayText = QString("%1[%2] %3 %4").arg(prefix, timestamp, line.toolName, line.message);
    } else {
        displayText = QString("%1[%2] %3").arg(prefix, timestamp, line.message);
    }

    auto* item = new QListWidgetItem(displayText, m_list);
    item->setForeground(color);
    {
        QFont font = QFontDatabase::systemFont(QFontDatabase::FixedFont);
        font.setPointSize(11);
        item->setFont(font);
    }

    m_list->scrollToBottom();
}

void ZoneFeed::clear() {
    m_list->clear();
    m_emptyLabel->show();
}

} // namespace kala