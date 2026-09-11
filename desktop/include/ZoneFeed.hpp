#pragma once

#include <QWidget>
#include <QVBoxLayout>
#include <QListWidget>
#include <QLabel>
#include "Types.hpp"

namespace kala {

class ZoneFeed : public QWidget {
    Q_OBJECT
public:
    explicit ZoneFeed(QWidget* parent = nullptr);
    void addToolLine(const ToolLine& line);
    void clear();

private:
    void setupUi();
    void applyStyle();

    QListWidget* m_list = nullptr;
    QLabel* m_emptyLabel = nullptr;
};

} // namespace kala