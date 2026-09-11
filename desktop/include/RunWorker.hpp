#pragma once

#include <QObject>
#include <QProcess>
#include <QString>
#include <QJsonObject>
#include "Types.hpp"

namespace kala {

class RunWorker : public QObject {
    Q_OBJECT
public:
    explicit RunWorker(QObject* parent = nullptr);
    void run(const QString& brief, const QString& starter, const QString& sessionId);
    void cancel();

signals:
    void started();
    void toolLine(const ToolLine& line);
    void finished(bool success, const QString& output, const QString& error);
    void sessionInfoChanged(const SessionInfo& info);
    void marginData(const MarginData& data);
    void statusData(const StatusRuleData& data);
    void stepPreview(const QString& stepPath);

private slots:
    void onReadyReadStandardOutput();
    void onReadyReadStandardError();
    void onFinished(int exitCode, QProcess::ExitStatus exitStatus);
    void onErrorOccurred(QProcess::ProcessError error);

private:
    QProcess* m_process = nullptr;
    QString m_buffer;
    QString m_currentSessionId;
    void parseJsonLine(const QString& line);
    void emitMockData(); // For prototype demo
};

} // namespace kala