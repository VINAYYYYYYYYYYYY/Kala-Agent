#include "ZoneSheet.hpp"
#include <QFont>
#include <QFontDatabase>
#include <QPainter>
#include <QPen>
#include <QLinearGradient>

namespace kala {

ZoneSheet::ZoneSheet(QWidget* parent) : QWidget(parent) {
    setupUi();
    applyStyle();
    setMinimumSize(400, 300);
    setMouseTracking(true);
}

void ZoneSheet::setupUi() {
    auto* layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setAlignment(Qt::AlignCenter);

    QFont monoFont = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    monoFont.setPointSize(12);

    m_emptyLabel = new QLabel(this);
    m_emptyLabel->setFont(monoFont);
    m_emptyLabel->setAlignment(Qt::AlignCenter);
    m_emptyLabel->setWordWrap(true);
    m_emptyLabel->setText("Sheet is clear.\nStart with \u00bfL-bracket\u00bf \u2014 mm brief, then STEP lands here.");
    m_emptyLabel->setObjectName("emptyLabel");
    layout->addWidget(m_emptyLabel);

    m_messageLabel = new QLabel(this);
    m_messageLabel->setFont(monoFont);
    m_messageLabel->setAlignment(Qt::AlignCenter);
    m_messageLabel->setWordWrap(true);
    m_messageLabel->setObjectName("messageLabel");
    m_messageLabel->hide();
    layout->addWidget(m_messageLabel);
}

void ZoneSheet::applyStyle() {
    setObjectName("sheet");
    setStyleSheet(R"(
        #sheet {
            background-color: #12151A;
            border: 1px solid #1E2329;
            border-radius: 0px;
        }
        #emptyLabel {
            color: #4A525A;
        }
        #messageLabel {
            color: #5B9FD4;
        }
    )");
}

void ZoneSheet::paintEvent(QPaintEvent* event) {
    QWidget::paintEvent(event);

    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing, false);

    if (!m_stepPixmap.isNull()) {
        QRect targetRect = rect().marginsRemoved(QMargins(8, 8, 8, 8));
        QPixmap scaled = m_stepPixmap.scaled(targetRect.size(), Qt::KeepAspectRatio, Qt::SmoothTransformation);
        QPoint pos = targetRect.center() - scaled.rect().center();
        painter.drawPixmap(pos, scaled);
    }

    if (m_isEmpty || !m_stepPixmap.isNull()) {
        drawGrid(painter);
        drawRegistrationMarks(painter);
    }

    if (m_isEmpty && m_message.isEmpty()) {
        drawEmptyState(painter);
    }
}

void ZoneSheet::resizeEvent(QResizeEvent* event) {
    QWidget::resizeEvent(event);
    update();
}

void ZoneSheet::drawGrid(QPainter& painter) {
    QPen gridPen(QColor(0x1A, 0x1F, 0x26), 1);
    painter.setPen(gridPen);

    const QRect& r = rect();
    for (int x = r.left(); x <= r.right(); x += GRID_SIZE) {
        painter.drawLine(x, r.top(), x, r.bottom());
    }
    for (int y = r.top(); y <= r.bottom(); y += GRID_SIZE) {
        painter.drawLine(r.left(), y, r.right(), y);
    }
}

void ZoneSheet::drawRegistrationMarks(QPainter& painter) {
    QPen markPen(QColor(0x5B, 0x9F, 0xD4, 60), 1);
    painter.setPen(markPen);

    const QRect& r = rect();
    int markLen = 12;
    int margin = 8;

    painter.drawLine(r.left() + margin, r.top() + margin, r.left() + margin + markLen, r.top() + margin);
    painter.drawLine(r.left() + margin, r.top() + margin, r.left() + margin, r.top() + margin + markLen);

    painter.drawLine(r.right() - margin, r.top() + margin, r.right() - margin - markLen, r.top() + margin);
    painter.drawLine(r.right() - margin, r.top() + margin, r.right() - margin, r.top() + margin + markLen);

    painter.drawLine(r.left() + margin, r.bottom() - margin, r.left() + margin + markLen, r.bottom() - margin);
    painter.drawLine(r.left() + margin, r.bottom() - margin, r.left() + margin, r.bottom() - margin - markLen);

    painter.drawLine(r.right() - margin, r.bottom() - margin, r.right() - margin - markLen, r.bottom() - margin);
    painter.drawLine(r.right() - margin, r.bottom() - margin, r.right() - margin, r.bottom() - margin - markLen);
}

void ZoneSheet::drawEmptyState(QPainter& painter) {
    QFont font = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    font.setPointSize(11);
    painter.setFont(font);
    painter.setPen(QColor(0x4A, 0x52, 0x5A));

    QString text = "Sheet is clear.  Start with L-bracket \u2014 mm brief, then STEP lands here.";
    QRect textRect = rect().marginsRemoved(QMargins(40, 40, 40, 40));
    painter.drawText(textRect, Qt::AlignCenter | Qt::TextWordWrap, text);
}

void ZoneSheet::setStepPreview(const QString& stepPath) {
    m_stepPixmap = QPixmap(stepPath);
    m_isEmpty = m_stepPixmap.isNull();
    m_emptyLabel->setVisible(m_isEmpty);
    update();
}

void ZoneSheet::setEmptyState(bool empty) {
    m_isEmpty = empty;
    m_emptyLabel->setVisible(empty);
    update();
}

void ZoneSheet::setMessage(const QString& message) {
    m_message = message;
    m_messageLabel->setText(message);
    m_messageLabel->setVisible(!message.isEmpty());
    update();
}

} // namespace kala