#include <QApplication>
#include <QSurfaceFormat>
#include <QDir>
#include <QCoreApplication>
#include "MainWindow.hpp"

int main(int argc, char* argv[]) {
    QApplication::setHighDpiScaleFactorRoundingPolicy(Qt::HighDpiScaleFactorRoundingPolicy::PassThrough);

    QApplication app(argc, argv);
    app.setApplicationName("Kala");
    app.setApplicationVersion("0.1.0");
    app.setOrganizationName("Kala");
    app.setOrganizationDomain("kala.dev");

    QSurfaceFormat format;
    format.setVersion(3, 3);
    format.setProfile(QSurfaceFormat::CoreProfile);
    format.setDepthBufferSize(24);
    format.setSamples(4);
    QSurfaceFormat::setDefaultFormat(format);

    QDir::setCurrent(QCoreApplication::applicationDirPath());

    kala::MainWindow window;
    window.show();

    return app.exec();
}