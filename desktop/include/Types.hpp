#pragma once

#include <QString>
#include <QJsonObject>
#include <QVector>
#include <optional>

namespace kala {

struct SessionInfo {
    QString sessionId;
    QString projectName;
    QString procedureId;
    QString plannerModel;
    bool freeCadConnected = false;
    QString units = "mm";
    QString status = "idle";
};

struct ToolLine {
    enum class Type { Info, ToolCall, ToolResult, Error, Summary };
    Type type = Type::Info;
    QString toolName;
    QString message;
    QString timestamp;
    bool isComplete = true;
};

struct ComposerState {
    QString briefText;
    bool hasProviderKey = false;
    QString currentStarter = "L-bracket";
    bool isRunning = false;
};

struct ExportInfo {
    QString stepPath;
    QString stlPath;
    QString assemblyPath;
    bool hasStep = false;
    bool hasStl = false;
    bool hasAssembly = false;
};

struct MarginData {
    SessionInfo session;
    ExportInfo exportInfo;
    QVector<QString> procedureSteps;
    int currentStepIndex = -1;
    QString freeCadStatus;
    QString toolsStatus;
};

struct StatusRuleData {
    QString units = "mm";
    QString stepStatus = "STEP: none";
    QString playbookStatus = "playbook: gated";
    QString backendStatus = "backend: disconnected";
    QString partsStatus = "parts: off";
};

} // namespace kala