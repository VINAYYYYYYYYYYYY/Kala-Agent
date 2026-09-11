#pragma once

#include <QWidget>
#include <QLabel>
#include <QVBoxLayout>
#include <QPaintEvent>
#include <QPainter>
#include <QPixmap>

namespace kala {

class ZoneSheet : public QWidget {
    Q_OBJECT
public:
    explicit ZoneSheet(QWidget* parent = nullptr);
    void setStepPreview(const QString& stepPath);
    void setEmptyState(bool empty);
    void setMessage(const QString& message);

protected:
    void paintEvent(QPaintEvent* event) override;
    void resizeEvent(QResizeEvent* event) override;

private:
    void setupUi();
    void applyStyle();
    void drawGrid(QPainter& painter);
    void drawRegistrationMarks(QPainter& painter);
    void drawEmptyState(QPainter& painter);

    QLabel* m_emptyLabel = nullptr;
    QLabel* m_messageLabel = nullptr;
    QPixmap m_stepPixmap;
    bool m_isEmpty = true;
    QString m_message;
    static constexpr int GRID_SIZE = 20;
};

} // namespace kala