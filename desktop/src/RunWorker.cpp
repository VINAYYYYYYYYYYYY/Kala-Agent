#include "RunWorker.hpp"
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonArray>
#include <QDateTime>
#include <QTimer>
#include <QDir>

namespace kala {

RunWorker::RunWorker(QObject* parent) : QObject(parent) {
    m_process = new QProcess(this);
    connect(m_process, &QProcess::readyReadStandardOutput, this, &RunWorker::onReadyReadStandardOutput);
    connect(m_process, &QProcess::readyReadStandardError, this, &RunWorker::onReadyReadStandardError);
    connect(m_process, QOverload<int, QProcess::ExitStatus>::of(&QProcess::finished),
            this, &RunWorker::onFinished);
    connect(m_process, QOverload<QProcess::ProcessError>::of(&QProcess::errorOccurred),
            this, &RunWorker::onErrorOccurred);
}

void RunWorker::run(const QString& brief, const QString& starter, const QString& sessionId) {
    m_currentSessionId = sessionId.isEmpty() ? QUuid::createUuid().toString(QUuid::WithoutBraces).left(8) : sessionId;
    m_buffer.clear();

    emit started();

    ToolLine startLine;
    startLine.type = ToolLine::Type::Info;
    startLine.message = QString("Starting session %1 with starter: %2").arg(m_currentSessionId, starter);
    startLine.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
    emit toolLine(startLine);

    QStringList args;
    args << "run" << "--json" << "--brief" << brief;
    if (starter != "custom") {
        args << "--starter" << starter;
    }
    args << "--session" << m_currentSessionId;

    QString kalaCmd = "kala";
    if (!QFileInfo::exists("/usr/local/bin/kala") && !QFileInfo::exists("/usr/bin/kala")) {
        kalaCmd = QDir::currentPath() + "/../.venv/bin/kala";
    }

    emit toolLine(ToolLine{ToolLine::Type::ToolCall, "kala", QString("Running: %1 %2").arg(kalaCmd, args.join(" ")), QDateTime::currentDateTime().toString("HH:mm:ss.zzz")});

    m_process->start(kalaCmd, args);

    if (!m_process->waitForStarted(5000)) {
        ToolLine errLine;
        errLine.type = ToolLine::Type::Error;
        errLine.message = "Failed to start kala process. Is 'kala' in PATH?";
        errLine.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        emit toolLine(errLine);
        emit finished(false, "", "Process failed to start");
        return;
    }

    SessionInfo sessionInfo;
    sessionInfo.sessionId = m_currentSessionId;
    sessionInfo.procedureId = starter;
    sessionInfo.status = "running";
    emit sessionInfoChanged(sessionInfo);
}

void RunWorker::cancel() {
    if (m_process->state() != QProcess::NotRunning) {
        m_process->terminate();
        if (!m_process->waitForFinished(2000)) {
            m_process->kill();
        }
    }
}

void RunWorker::onReadyReadStandardOutput() {
    m_buffer += m_process->readAllStandardOutput();

    while (true) {
        int newlinePos = m_buffer.indexOf('\n');
        if (newlinePos < 0) break;

        QString line = m_buffer.left(newlinePos).trimmed();
        m_buffer = m_buffer.mid(newlinePos + 1);

        if (!line.isEmpty()) {
            parseJsonLine(line);
        }
    }
}

void RunWorker::onReadyReadStandardError() {
    QString err = m_process->readAllStandardError();
    if (!err.trimmed().isEmpty()) {
        ToolLine line;
        line.type = ToolLine::Type::Error;
        line.message = err.trimmed();
        line.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        emit toolLine(line);
    }
}

void RunWorker::parseJsonLine(const QString& line) {
    QJsonParseError error;
    QJsonDocument doc = QJsonDocument::fromJson(line.toUtf8(), &error);
    if (error.error != QJsonParseError::NoError) {
        ToolLine tl;
        tl.type = ToolLine::Type::Info;
        tl.message = line;
        tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        emit toolLine(tl);
        return;
    }

    QJsonObject obj = doc.object();
    QString type = obj["type"].toString();

    if (type == "tool_call") {
        ToolLine tl;
        tl.type = ToolLine::Type::ToolCall;
        tl.toolName = obj["tool"].toString();
        tl.message = obj["args"].toString();
        tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        emit toolLine(tl);
    } else if (type == "tool_result") {
        ToolLine tl;
        tl.type = ToolLine::Type::ToolResult;
        tl.toolName = obj["tool"].toString();
        tl.message = obj["result"].toString();
        tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        emit toolLine(tl);
    } else if (type == "summary") {
        ToolLine tl;
        tl.type = ToolLine::Type::Summary;
        tl.message = obj["message"].toString();
        tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        emit toolLine(tl);
    } else if (type == "session") {
        SessionInfo info;
        info.sessionId = obj["session_id"].toString();
        info.procedureId = obj["procedure_id"].toString();
        info.plannerModel = obj["planner"].toString();
        info.freeCadConnected = obj["freecad_connected"].toBool();
        info.status = obj["status"].toString();
        emit sessionInfoChanged(info);
    } else if (type == "margin") {
        MarginData data;
        data.session.sessionId = obj["session_id"].toString();
        data.session.status = obj["status"].toString();
        data.session.plannerModel = obj["planner"].toString();
        data.session.freeCadConnected = obj["freecad_connected"].toBool();
        data.freeCadStatus = obj["freecad_status"].toString();
        data.toolsStatus = obj["tools_status"].toString();
        data.exportInfo.stepPath = obj["step_path"].toString();
        data.exportInfo.stlPath = obj["stl_path"].toString();
        data.exportInfo.hasStep = !data.exportInfo.stepPath.isEmpty();
        data.exportInfo.hasStl = !data.exportInfo.stlPath.isEmpty();
        QJsonArray steps = obj["procedure_steps"].toArray();
        for (const auto& v : steps) data.procedureSteps.append(v.toString());
        data.currentStepIndex = obj["current_step"].toInt(-1);
        emit marginData(data);
    } else if (type == "status") {
        StatusRuleData data;
        data.units = obj["units"].toString("mm");
        data.stepStatus = obj["step_status"].toString("STEP: none");
        data.playbookStatus = obj["playbook_status"].toString("playbook: gated");
        data.backendStatus = obj["backend_status"].toString("backend: disconnected");
        data.partsStatus = obj["parts_status"].toString("parts: off");
        emit statusData(data);
    } else if (type == "step_preview") {
        emit stepPreview(obj["path"].toString());
    } else if (type == "error") {
        ToolLine tl;
        tl.type = ToolLine::Type::Error;
        tl.message = obj["message"].toString();
        tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        emit toolLine(tl);
    }
}

void RunWorker::onFinished(int exitCode, QProcess::ExitStatus exitStatus) {
    QString stdoutOutput = m_process->readAllStandardOutput();
    QString stderrOutput = m_process->readAllStandardError();

    bool success = (exitStatus == QProcess::NormalExit && exitCode == 0);

    ToolLine tl;
    tl.type = success ? ToolLine::Type::Summary : ToolLine::Type::Error;
    tl.message = success ? "Procedure completed successfully" : QString("Process exited with code %1").arg(exitCode);
    tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
    emit toolLine(tl);

    SessionInfo sessionInfo;
    sessionInfo.sessionId = m_currentSessionId;
    sessionInfo.status = success ? "completed" : "failed";
    emit sessionInfoChanged(sessionInfo);

    emit finished(success, stdoutOutput, stderrOutput);
}

void RunWorker::onErrorOccurred(QProcess::ProcessError error) {
    QString errMsg;
    switch (error) {
        case QProcess::FailedToStart: errMsg = "Failed to start process"; break;
        case QProcess::Crashed: errMsg = "Process crashed"; break;
        case QProcess::Timedout: errMsg = "Process timed out"; break;
        case QProcess::WriteError: errMsg = "Write error"; break;
        case QProcess::ReadError: errMsg = "Read error"; break;
        case QProcess::UnknownError: errMsg = "Unknown error"; break;
        default: errMsg = "Process error";
    }

    ToolLine tl;
    tl.type = ToolLine::Type::Error;
    tl.message = errMsg;
    tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
    emit toolLine(tl);
}

void RunWorker::emitMockData() {
    QTimer::singleShot(500, this, [this]() {
        ToolLine tl;
        tl.type = ToolLine::Type::ToolCall;
        tl.toolName = "planner.plan";
        tl.message = "Planning L-bracket procedure";
        tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        emit toolLine(tl);
    });

    QTimer::singleShot(1000, this, [this]() {
        ToolLine tl;
        tl.type = ToolLine::Type::ToolResult;
        tl.toolName = "planner.plan";
        tl.message = "Plan: sketch base, extrude, fillet, holes";
        tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        emit toolLine(tl);
    });

    QTimer::singleShot(1500, this, [this]() {
        ToolLine tl;
        tl.type = ToolLine::Type::ToolCall;
        tl.toolName = "freecad.build";
        tl.message = "Building in FreeCAD...";
        tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        emit toolLine(tl);
    });

    QTimer::singleShot(2500, this, [this]() {
        ToolLine tl;
        tl.type = ToolLine::Type::ToolResult;
        tl.toolName = "freecad.build";
        tl.message = "Part created: L_bracket.step";
        tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        emit toolLine(tl);

        emit stepPreview("L_bracket.step");
    });

    QTimer::singleShot(3000, this, [this]() {
        emit finished(true, "Success", "");
    });
}

} // namespace kala