#include "mainwindow.h"

#include <QApplication>
#include <QStyle>
#include <QStyleFactory>

#ifndef SW_EXPLORER_VERSION
#define SW_EXPLORER_VERSION "0.1.0"
#endif

int main(int argc, char *argv[]) {
    QApplication app(argc, argv);
    app.setApplicationName("sw-explorer");
    app.setApplicationVersion(QStringLiteral(SW_EXPLORER_VERSION));
    if (QStyleFactory::keys().contains("windowsvista", Qt::CaseInsensitive)) {
        if (QStyle *style = QStyleFactory::create("windowsvista")) {
            app.setStyle(style);
        }
    }
    MainWindow w;
    w.show();
    return app.exec();
}
