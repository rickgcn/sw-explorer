#include "MainWindow.h"

#include "HierarchySnapshot.h"
#include "InspectorSnapshot.h"

#include <QApplication>

int main(int argc, char *argv[])
{
    // Snapshots and the request kind cross threads through queued
    // signals.
    qRegisterMetaType<HierarchyKind>("HierarchyKind");
    qRegisterMetaType<HierarchySnapshot>("HierarchySnapshot");
    qRegisterMetaType<ProductDetailSnapshot>("ProductDetailSnapshot");
    qRegisterMetaType<ImageDetailSnapshot>("ImageDetailSnapshot");
    qRegisterMetaType<SubsystemDetailSnapshot>("SubsystemDetailSnapshot");

    QApplication app(argc, argv);

    MainWindow window;
    window.show();

    return QApplication::exec();
}
