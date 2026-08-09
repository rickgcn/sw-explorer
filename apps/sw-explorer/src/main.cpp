#include "MainWindow.h"

#include "HierarchySnapshot.h"

#include <QApplication>

int main(int argc, char *argv[])
{
    // The hierarchy snapshot crosses threads through a queued signal.
    qRegisterMetaType<HierarchySnapshot>("HierarchySnapshot");

    QApplication app(argc, argv);

    MainWindow window;
    window.show();

    return QApplication::exec();
}
