#pragma once

#include <QWidget>
#include <QVBoxLayout>
#include <QTextEdit>
#include <QPushButton>
#include <QComboBox>
#include <QLabel>
#include "Types.hpp"

namespace kala {

class ZoneComposer : public QWidget {
    Q_OBJECT
public:
    explicit ZoneComposer(QWidget* parent = nullptr);
    void setState(const ComposerState& state);
    QString getBriefText() const;

signals:
    void runRequested(const QString& brief, const QString& starter);
    void starterChanged(const QString& starterId);

private:
    void setupUi();
    void applyStyle();

    QComboBox* m_starterCombo = nullptr;
    QTextEdit* m_briefEdit = nullptr;
    QPushButton* m_runButton = nullptr;
    QLabel* m_providerLabel = nullptr;
    bool m_isRunning = false;
};

} // namespace kala