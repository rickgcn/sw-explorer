#include "MainWindow.h"

#include "EntrySnapshot.h"
#include "ExtractionSnapshot.h"
#include "HardwareSnapshot.h"
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
    qRegisterMetaType<EntryListSnapshot>("EntryListSnapshot");
    qRegisterMetaType<EntryDetailSnapshot>("EntryDetailSnapshot");
    qRegisterMetaType<EntryKey>("EntryKey");
    qRegisterMetaType<HardwareProfileSnapshot>("HardwareProfileSnapshot");
    qRegisterMetaType<HardwareCandidatesSnapshot>("HardwareCandidatesSnapshot");
    qRegisterMetaType<SelectionSnapshot>("SelectionSnapshot");
    qRegisterMetaType<ExtractionRequestSnapshot>("ExtractionRequestSnapshot");
    qRegisterMetaType<ExtractionPlanSnapshot>("ExtractionPlanSnapshot");
    qRegisterMetaType<ExtractionReportSnapshot>("ExtractionReportSnapshot");

    QApplication app(argc, argv);

    MainWindow window;
    window.show();

    return QApplication::exec();
}
