#pragma once

#include <QWidget>
#include <QLabel>
#include <QHBoxLayout>
#include "Types.hpp"

namespace kala {

class ZoneTitleBlock : public QWidget {
    Q_OBJECT
public:
    explicit ZoneTitleBlock(QWidget* parent = nullptr);
    void updateSession(const SessionInfo& info);

private:
    void setupUi();
    void applyStyle();

    QLabel* m_appLabel = nullptr;
    QLabel* m_sessionStatus = nullptr;
    QLabel* m_procedureId = nullptr;
    QLabel* m_plannerModel = nullptr;
    QLabel* m_freeCadSync = nullptr;
    QLabel* m_unitsLabel = nullptr;
    QLabel* m_revisionLabel = nullptr;
};

} // namespace kala