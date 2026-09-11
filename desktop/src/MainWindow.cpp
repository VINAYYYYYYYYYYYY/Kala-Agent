#include "MainWindow.hpp"
#include <QApplication>
#include <QScreen>
#include <QThread>
#include <QUuid>
#include <QTimer>
#include <QMessageBox>
#include <QFileDialog>
#include <QDesktopServices>
#include <QUrl>
#include <QFontDatabase>
#include <QDateTime>
#include <QFile>

namespace kala {

MainWindow::MainWindow(QWidget* parent) : QMainWindow(parent) {
    setupUi();
    applyGlobalStyle();
    connectSignals();

    m_worker = new RunWorker();
    m_workerThread = new QThread(this);
    m_worker->moveToThread(m_workerThread);
    m_workerThread->start();

    m_currentSessionId = generateSessionId();

    SessionInfo initialSession;
    initialSession.sessionId = m_currentSessionId;
    initialSession.status = "idle";
    m_titleBlock->updateSession(initialSession);

    ComposerState initialComposer;
    initialComposer.hasProviderKey = false;
    m_composer->setState(initialComposer);

    MarginData initialMargin;
    initialMargin.session = initialSession;
    m_margin->updateData(initialMargin);

    StatusRuleData initialStatus;
    m_statusRule->updateData(initialStatus);

    resize(1400, 900);
    QScreen* screen = QApplication::primaryScreen();
    if (screen) {
        QRect geo = screen->availableGeometry();
        move((geo.width() - width()) / 2, (geo.height() - height()) / 2);
    }
}

void MainWindow::setupUi() {
    setWindowTitle("Kala \u2014 Drafting Board");
    setMinimumSize(1000, 700);

    auto* centralWidget = new QWidget(this);
    setCentralWidget(centralWidget);

    auto* mainLayout = new QVBoxLayout(centralWidget);
    mainLayout->setContentsMargins(0, 0, 0, 0);
    mainLayout->setSpacing(0);

    m_titleBlock = new ZoneTitleBlock(this);
    mainLayout->addWidget(m_titleBlock);

    auto* contentSplitter = new QSplitter(Qt::Horizontal, this);
    contentSplitter->setChildrenCollapsible(false);
    contentSplitter->setHandleWidth(1);
    contentSplitter->setStyleSheet("QSplitter::handle { background-color: #1E2329; }");

    auto* sheetFeedWidget = new QWidget(this);
    auto* sheetFeedLayout = new QVBoxLayout(sheetFeedWidget);
    sheetFeedLayout->setContentsMargins(0, 0, 0, 0);
    sheetFeedLayout->setSpacing(0);

    m_sheet = new ZoneSheet(this);
    sheetFeedLayout->addWidget(m_sheet, 1);

    m_feed = new ZoneFeed(this);
    sheetFeedLayout->addWidget(m_feed);

    contentSplitter->addWidget(sheetFeedWidget);

    m_margin = new ZoneMargin(this);
    contentSplitter->addWidget(m_margin);

    contentSplitter->setStretchFactor(0, 1);
    contentSplitter->setStretchFactor(1, 0);
    contentSplitter->setSizes({1000, 260});

    mainLayout->addWidget(contentSplitter, 1);

    m_composer = new ZoneComposer(this);
    mainLayout->addWidget(m_composer);

    m_statusRule = new ZoneStatusRule(this);
    mainLayout->addWidget(m_statusRule);
}

void MainWindow::applyGlobalStyle() {
    setStyleSheet(R"(
        QMainWindow {
            background-color: #0B0D10;
        }
        QWidget {
            color: #E8EDF2;
            font-family: "IBM Plex Sans", "Inter", "Segoe UI", sans-serif;
            font-size: 12px;
        }
        QToolTip {
            background-color: #161A20;
            border: 1px solid #1E2329;
            color: #A8B0BA;
            padding: 4px 8px;
        }
    )");

    QFont defaultFont = QFontDatabase::systemFont(QFontDatabase::GeneralFont);
    defaultFont.setPointSize(12);
    defaultFont.setHintingPreference(QFont::PreferFullHinting);
    qApp->setFont(defaultFont);
}

void MainWindow::connectSignals() {
    connect(m_composer, &ZoneComposer::runRequested, this, &MainWindow::onRunRequested);
    connect(m_composer, &ZoneComposer::starterChanged, this, &MainWindow::onStarterChanged);
    connect(m_margin, &ZoneMargin::procedureChanged, this, &MainWindow::onProcedureChanged);
    connect(m_margin, &ZoneMargin::exportStepRequested, this, &MainWindow::onExportStepRequested);
    connect(m_margin, &ZoneMargin::exportStlRequested, this, &MainWindow::onExportStlRequested);
    connect(m_margin, &ZoneMargin::openInFreeCadRequested, this, &MainWindow::onOpenInFreeCadRequested);

    connect(m_worker, &RunWorker::started, this, [this]() {
        m_composerState.isRunning = true;
        updateComposerState();
    });
    connect(m_worker, &RunWorker::toolLine, this, &MainWindow::onToolLine);
    connect(m_worker, &RunWorker::finished, this, &MainWindow::onWorkerFinished);
    connect(m_worker, &RunWorker::sessionInfoChanged, this, &MainWindow::onSessionInfo);
    connect(m_worker, &RunWorker::marginData, this, &MainWindow::onMarginData);
    connect(m_worker, &RunWorker::statusData, this, &MainWindow::onStatusData);
    connect(m_worker, &RunWorker::stepPreview, this, &MainWindow::onStepPreview);
}

void MainWindow::onRunRequested(const QString& brief, const QString& starter) {
    if (brief.trimmed().isEmpty()) {
        QMessageBox::warning(this, "Empty Brief", "Please describe the part in mm before running.");
        return;
    }

    m_feed->clear();
    m_sheet->setEmptyState(true);
    m_sheet->setMessage("");

    m_worker->run(brief, starter, m_currentSessionId);
}

void MainWindow::onToolLine(const ToolLine& line) {
    m_feed->addToolLine(line);
}

void MainWindow::onWorkerFinished(bool success, const QString& output, const QString& error) {
    m_composerState.isRunning = false;
    updateComposerState();

    if (!success && !error.isEmpty()) {
        ToolLine tl;
        tl.type = ToolLine::Type::Error;
        tl.message = error;
        tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        m_feed->addToolLine(tl);
    }
}

void MainWindow::onSessionInfo(const SessionInfo& info) {
    m_titleBlock->updateSession(info);
    m_marginData.session = info;
    m_margin->updateData(m_marginData);
}

void MainWindow::onMarginData(const MarginData& data) {
    m_marginData = data;
    m_margin->updateData(data);
}

void MainWindow::onStatusData(const StatusRuleData& data) {
    m_statusData = data;
    m_statusRule->updateData(data);
}

void MainWindow::onStepPreview(const QString& stepPath) {
    m_sheet->setEmptyState(false);
    m_sheet->setStepPreview(stepPath);
    m_sheet->setMessage(QString("STEP loaded: %1").arg(stepPath));
}

void MainWindow::onStarterChanged(const QString& starterId) {
    m_composerState.currentStarter = starterId;
}

void MainWindow::onProcedureChanged(const QString& procedureId) {
    m_composerState.currentStarter = procedureId;
    int idx = m_composer->findChild<QComboBox*>("starterCombo")->findData(procedureId);
    if (idx >= 0) {
        m_composer->findChild<QComboBox*>("starterCombo")->setCurrentIndex(idx);
    }
}

void MainWindow::onExportStepRequested() {
    if (m_marginData.exportInfo.stepPath.isEmpty()) return;
    QString dest = QFileDialog::getSaveFileName(this, "Export STEP", m_marginData.exportInfo.stepPath, "STEP Files (*.step *.stp)");
    if (!dest.isEmpty()) {
        QFile::copy(m_marginData.exportInfo.stepPath, dest);
        ToolLine tl;
        tl.type = ToolLine::Type::Summary;
        tl.message = QString("Exported STEP to %1").arg(dest);
        tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        m_feed->addToolLine(tl);
    }
}

void MainWindow::onExportStlRequested() {
    if (m_marginData.exportInfo.stlPath.isEmpty()) return;
    QString dest = QFileDialog::getSaveFileName(this, "Export STL", m_marginData.exportInfo.stlPath, "STL Files (*.stl)");
    if (!dest.isEmpty()) {
        QFile::copy(m_marginData.exportInfo.stlPath, dest);
        ToolLine tl;
        tl.type = ToolLine::Type::Summary;
        tl.message = QString("Exported STL to %1").arg(dest);
        tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
        m_feed->addToolLine(tl);
    }
}

void MainWindow::onOpenInFreeCadRequested() {
    ToolLine tl;
    tl.type = ToolLine::Type::Info;
    tl.message = "Opening in FreeCAD... (external process)";
    tl.timestamp = QDateTime::currentDateTime().toString("HH:mm:ss.zzz");
    m_feed->addToolLine(tl);

    if (!m_marginData.exportInfo.stepPath.isEmpty()) {
        QDesktopServices::openUrl(QUrl::fromLocalFile(m_marginData.exportInfo.stepPath));
    }
}

void MainWindow::updateComposerState() {
    m_composer->setState(m_composerState);
}

QString MainWindow::generateSessionId() {
    return QUuid::createUuid().toString(QUuid::WithoutBraces).left(8);
}

} // namespace kala