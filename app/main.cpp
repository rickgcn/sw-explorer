#include "mainwindow.h"

#include <QApplication>
#include <QStyle>
#include <QStyleFactory>

int main(int argc, char *argv[]) {
    QApplication app(argc, argv);
    if (QStyleFactory::keys().contains("windowsvista", Qt::CaseInsensitive)) {
        if (QStyle *style = QStyleFactory::create("windowsvista")) {
            app.setStyle(style);
        }
    }
    MainWindow w;
    w.show();
    return app.exec();
}
