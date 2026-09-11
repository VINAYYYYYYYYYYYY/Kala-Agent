#pragma once

#include <QWidget>
#include <QVBoxLayout>
#include <QLabel>
#include <QComboBox>
#include <QListWidget>
#include <QPushButton>
#include <QGroupBox>
#include "Types.hpp"

namespace kala {

class ZoneMargin : public QWidget {
    Q_OBJECT
public:
    explicit ZoneMargin(QWidget* parent = nullptr);
    void updateData(const MarginData& data);

signals:
    void procedureChanged(const QString& procedureId);
    void exportStepRequested();
    void exportStlRequested();
    void openInFreeCadRequested();

private:
    void setupUi();
    void applyStyle();
    void createSessionGroup(QVBoxLayout* layout);
    void createFreeCadGroup(QVBoxLayout* layout);
    void createExportGroup(QVBoxLayout* layout);
    void createProcedureGroup(QVBoxLayout* layout);

    QLabel* m_sessionStatus = nullptr;
    QLabel* m_backendStatus = nullptr;
    QLabel* m_freeCadStatus = nullptr;
    QLabel* m_toolsStatus = nullptr;
    QLabel* m_exportPath = nullptr;
    QComboBox* m_procedureCombo = nullptr;
    QListWidget* m_stepList = nullptr;
    QPushButton* m_exportStepBtn = nullptr;
    QPushButton* m_exportStlBtn = nullptr;
    QPushButton* m_openFreeCadBtn = nullptr;
};

} // namespace kala