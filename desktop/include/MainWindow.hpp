#pragma once

#include <QMainWindow>
#include <QSplitter>
#include <QVBoxLayout>
#include <QHBoxLayout>
#include "ZoneTitleBlock.hpp"
#include "ZoneSheet.hpp"
#include "ZoneMargin.hpp"
#include "ZoneFeed.hpp"
#include "ZoneComposer.hpp"
#include "ZoneStatusRule.hpp"
#include "RunWorker.hpp"
#include "Types.hpp"

namespace kala {

class MainWindow : public QMainWindow {
    Q_OBJECT
public:
    explicit MainWindow(QWidget* parent = nullptr);

private slots:
    void onRunRequested(const QString& brief, const QString& starter);
    void onToolLine(const ToolLine& line);
    void onWorkerFinished(bool success, const QString& output, const QString& error);
    void onSessionInfo(const SessionInfo& info);
    void onMarginData(const MarginData& data);
    void onStatusData(const StatusRuleData& data);
    void onStepPreview(const QString& stepPath);
    void onStarterChanged(const QString& starterId);
    void onProcedureChanged(const QString& procedureId);
    void onExportStepRequested();
    void onExportStlRequested();
    void onOpenInFreeCadRequested();

private:
    void setupUi();
    void applyGlobalStyle();
    void connectSignals();
    void updateComposerState();
    QString generateSessionId();

    ZoneTitleBlock* m_titleBlock = nullptr;
    ZoneSheet* m_sheet = nullptr;
    ZoneMargin* m_margin = nullptr;
    ZoneFeed* m_feed = nullptr;
    ZoneComposer* m_composer = nullptr;
    ZoneStatusRule* m_statusRule = nullptr;

    RunWorker* m_worker = nullptr;
    QThread* m_workerThread = nullptr;

    ComposerState m_composerState;
    MarginData m_marginData;
    StatusRuleData m_statusData;
    QString m_currentSessionId;
};

} // namespace kala