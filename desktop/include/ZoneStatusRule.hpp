#pragma once

#include <QWidget>
#include <QHBoxLayout>
#include <QLabel>
#include "Types.hpp"

namespace kala {

class ZoneStatusRule : public QWidget {
    Q_OBJECT
public:
    explicit ZoneStatusRule(QWidget* parent = nullptr);
    void updateData(const StatusRuleData& data);

private:
    void setupUi();
    void applyStyle();

    QLabel* m_unitsLabel = nullptr;
    QLabel* m_stepLabel = nullptr;
    QLabel* m_playbookLabel = nullptr;
    QLabel* m_backendLabel = nullptr;
    QLabel* m_partsLabel = nullptr;
};

} // namespace kala