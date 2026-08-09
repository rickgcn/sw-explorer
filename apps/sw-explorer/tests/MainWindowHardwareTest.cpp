#include "MainWindow.h"

#include <QAbstractItemModel>
#include <QApplication>
#include <QComboBox>
#include <QFile>
#include <QLabel>
#include <QLineEdit>
#include <QMessageBox>
#include <QSignalSpy>
#include <QStackedWidget>
#include <QStatusBar>
#include <QTableView>
#include <QTemporaryDir>
#include <QTest>
#include <QTimer>
#include <QToolButton>
#include <QTreeView>

#include "EntryBrowserWidget.h"
#include "HardwareProfileDialog.h"
#include "models/EntryTableModel.h"

// Drives the real MainWindow against the real Rust backend (through
// BackendWorker on its thread) with synthetic distributions carrying
// MACH attributes: the hardware profile lifecycle, the third request
// family, the selection overlay and the candidate suggestions.
//
// The distribution: `alpha` with usr/bin/Xsgi (mach-less),
// usr/lib/libGL.so (CPUBOARD=IP22), usr/lib/libX10.so (mach-less) in
// alpha.sw.unix / alpha.sw.gfx, plus two mach-less usr/share/conf
// duplicates; `beta` with a second usr/bin/Xsgi record. Object ids:
// 1 = alpha, 2 = alpha.sw, 3 = unix, 4 = gfx, 5 = beta, 6 = beta.sw,
// 7 = unix.
class MainWindowHardwareTest : public QObject
{
    Q_OBJECT

private slots:
    void initTestCase();
    void hardwareButtonIsAvailableAtStartup();
    void profileIsSavedWithoutDistribution();
    void commitTriggersSelectionForStoredProfile();
    void applyOnlyIssuesASelectionRequest();
    void toolbarSummaryAndTooltip();
    void clearingProfileDisablesOverlay();
    void staleSelectionResponseIsDropped();
    void profileChangeLeavesSearchResultsUntouched();
    void profileChangeLeavesHierarchyScopeUntouched();
    void switchingProfileOnlyChangesStatuses();
    void failedOpenKeepsTheWholeHardwareState();
    void successfulReopenKeepsProfileAndReselects();
    void hardwareButtonLocksDuringOpen();
    void candidatesRefreshAfterCommit();
    void staleCandidatesResponseIsDropped();
    void selectionErrorKeepsProfileAndFiles();
    void emptyValueProfileSelectsHeadlessRecord();
};

namespace {

bool writeHardwareDist(QTemporaryDir &dir)
{
    QFile alphaIdb(dir.filePath(QStringLiteral("alpha.idb")));
    if (!alphaIdb.open(QIODevice::WriteOnly)) {
        return false;
    }
    alphaIdb.write(
        "f 0755 root sys usr/bin/Xsgi src/Xsgi alpha.sw.unix sum(1) size(100) cmpsize(60)\n"
        "f 0644 root sys usr/lib/libGL.so src/libGL alpha.sw.gfx sum(2) size(200) cmpsize(0) mach(CPUBOARD=IP22)\n"
        "f 0644 root sys usr/lib/libX10.so src/libX10 alpha.sw.gfx sum(3) size(300) cmpsize(0)\n"
        "f 0644 root sys usr/share/conf src/conf1 alpha.sw.gfx sum(4) size(400) cmpsize(0)\n"
        "f 0644 root sys usr/share/conf src/conf2 alpha.sw.gfx sum(5) size(500) cmpsize(0)\n");
    alphaIdb.close();

    QFile betaIdb(dir.filePath(QStringLiteral("beta.idb")));
    if (!betaIdb.open(QIODevice::WriteOnly)) {
        return false;
    }
    betaIdb.write(
        "f 0755 root sys usr/bin/Xsgi src/Xsgi-beta beta.sw.unix sum(6) size(110) cmpsize(0)\n");
    return true;
}

// One product `gamma` with a single mach-less usr/bin/gamma record.
bool writeGammaDist(QTemporaryDir &dir)
{
    QFile idb(dir.filePath(QStringLiteral("gamma.idb")));
    if (!idb.open(QIODevice::WriteOnly)) {
        return false;
    }
    idb.write(
        "f 0755 root sys usr/bin/gamma src/gamma gamma.sw.unix sum(1) size(10) cmpsize(0)\n");
    return true;
}

// One product `gfx` with a mach-less record and one record carrying
// `mach(GFXBOARD=)` — the empty right-hand side real media use to
// restrict a record to headless boards.
bool writeEmptyValueDist(QTemporaryDir &dir)
{
    QFile idb(dir.filePath(QStringLiteral("gfx.idb")));
    if (!idb.open(QIODevice::WriteOnly)) {
        return false;
    }
    idb.write(
        "f 0755 root sys usr/bin/any src/any gfx.sw.unix sum(1) size(10) cmpsize(0)\n"
        "f 0755 root sys usr/bin/headless src/headless gfx.sw.unix sum(1) size(10) cmpsize(0) mach(GFXBOARD=)\n");
    return true;
}

QToolButton *hardwareButtonOf(MainWindow &window)
{
    return window.findChild<QToolButton *>(QStringLiteral("hardwareButton"));
}

QTreeView *treeOf(MainWindow &window)
{
    return window.findChild<QTreeView *>();
}

QTableView *entryTableOf(MainWindow &window)
{
    return window.findChild<QTableView *>(QStringLiteral("entryTable"));
}

QLabel *labelOf(MainWindow &window, const QString &objectName)
{
    return window.findChild<QLabel *>(objectName);
}

QString statusTextOf(MainWindow &window)
{
    return window.statusBar()->currentMessage();
}

HardwareProfileSnapshot ip22Profile()
{
    return {{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}};
}

// Opens the distribution and waits until the candidate is committed:
// the tree carries the new hierarchy and interaction is unlocked.
void loadDistribution(MainWindow &window, const QString &path)
{
    window.openDistributionRequested(path);
    QTRY_VERIFY(treeOf(window)->model()->rowCount() > 0);
    QTRY_VERIFY(treeOf(window)->isEnabled());
}

// Waits until the current selection request has been overlaid: the
// Status column becomes visible.
void waitForOverlay(MainWindow &window)
{
    QTRY_VERIFY(!entryTableOf(window)->isColumnHidden(EntryTableModel::StatusColumn));
}

QModelIndex productIndex(QAbstractItemModel *model, const QString &name)
{
    for (int row = 0; row < model->rowCount(); ++row) {
        const QModelIndex index = model->index(row, 0);
        if (model->data(index).toString() == name) {
            return index;
        }
    }
    return {};
}

// Runs `inspect` on the modal hardware profile dialog; the dialog is
// rejected afterwards. Returns whether the dialog showed up and the
// inspection passed.
template <typename F>
bool withHardwareDialog(MainWindow &window, F inspect)
{
    bool seen = false;
    bool ok = false;
    QTimer watcher;
    watcher.setInterval(50);
    QObject::connect(&watcher, &QTimer::timeout, [&] {
        auto *dialog =
            qobject_cast<HardwareProfileDialog *>(QApplication::activeModalWidget());
        if (dialog != nullptr && !seen) {
            seen = true;
            ok = inspect(dialog);
            dialog->reject();
        }
    });
    watcher.start();
    hardwareButtonOf(window)->click();
    watcher.stop();
    return seen && ok;
}

} // namespace

void MainWindowHardwareTest::initTestCase()
{
    qRegisterMetaType<HierarchyKind>("HierarchyKind");
    qRegisterMetaType<EntryListSnapshot>("EntryListSnapshot");
    qRegisterMetaType<HardwareProfileSnapshot>("HardwareProfileSnapshot");
    qRegisterMetaType<HardwareCandidatesSnapshot>("HardwareCandidatesSnapshot");
    qRegisterMetaType<SelectionSnapshot>("SelectionSnapshot");
}

void MainWindowHardwareTest::hardwareButtonIsAvailableAtStartup()
{
    MainWindow window;

    QToolButton *button = hardwareButtonOf(window);
    QVERIFY(button != nullptr);
    // No distribution loaded: the profile editor still opens.
    QVERIFY(button->isEnabled());
    QCOMPARE(button->text(), QStringLiteral("Hardware: Off"));

    window.show();
    QVERIFY(QTest::qWaitForWindowExposed(&window));
    QVERIFY(withHardwareDialog(window, [](HardwareProfileDialog *dialog) {
        return dialog->findChild<QTableView *>(QStringLiteral("profileTable")) != nullptr;
    }));
}

void MainWindowHardwareTest::profileIsSavedWithoutDistribution()
{
    MainWindow window;
    QSignalSpy selectionSpy(&window, &MainWindow::selectionRequested);

    window.applyHardwareProfile(ip22Profile());
    QCOMPARE(hardwareButtonOf(window)->text(), QStringLiteral("Hardware: IP22"));
    QCOMPARE(hardwareButtonOf(window)->toolTip(), QStringLiteral("CPUBOARD=IP22"));
    // Nothing to select against: no request, plain status text.
    QCOMPARE(selectionSpy.count(), 0);
    QCOMPARE(statusTextOf(window), QStringLiteral("No distribution loaded."));
}

void MainWindowHardwareTest::commitTriggersSelectionForStoredProfile()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    MainWindow window;
    // The profile comes first; the distribution arrives second.
    window.applyHardwareProfile(ip22Profile());
    loadDistribution(window, dir.path());

    // The commit evaluated the stored profile automatically.
    waitForOverlay(window);
    QCOMPARE(statusTextOf(window),
             QStringLiteral("Loaded 2 products · 6 selected records · 1 conflict groups"));

    // The overlay applies to whatever scope the user browses.
    treeOf(window)->setCurrentIndex(productIndex(treeOf(window)->model(), QStringLiteral("alpha")));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 5);
    QCOMPARE(labelOf(window, QStringLiteral("countLabel"))->text(),
             QStringLiteral("5 entries · 5 selected · 2 conflicted records"));
    QCOMPARE(entryTableOf(window)->model()->index(1, EntryTableModel::StatusColumn).data().toString(),
             QStringLiteral("Selected"));
}

void MainWindowHardwareTest::applyOnlyIssuesASelectionRequest()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());

    treeOf(window)->setCurrentIndex(productIndex(treeOf(window)->model(), QStringLiteral("alpha")));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 5);

    QSignalSpy entriesSpy(&window, &MainWindow::entriesRequested);
    QSignalSpy searchSpy(&window, &MainWindow::searchEntriesRequested);
    QSignalSpy detailSpy(&window, &MainWindow::detailRequested);
    QSignalSpy entryDetailSpy(&window, &MainWindow::entryDetailRequested);
    QSignalSpy selectionSpy(&window, &MainWindow::selectionRequested);

    window.applyHardwareProfile(ip22Profile());
    QCOMPARE(selectionSpy.count(), 1);
    const HardwareProfileSnapshot sent =
        qvariant_cast<HardwareProfileSnapshot>(selectionSpy.first().at(1));
    QCOMPARE(sent.size(), 1);
    QCOMPARE(sent.at(0).attribute, QStringLiteral("CPUBOARD"));
    QCOMPARE(sent.at(0).value, QStringLiteral("IP22"));

    // A profile change never reloads entries, searches or details.
    QCOMPARE(entriesSpy.count(), 0);
    QCOMPARE(searchSpy.count(), 0);
    QCOMPARE(detailSpy.count(), 0);
    QCOMPARE(entryDetailSpy.count(), 0);

    // While the request runs, the old overlay is already gone.
    QVERIFY(entryTableOf(window)->isColumnHidden(EntryTableModel::StatusColumn));
    QCOMPARE(entryTableOf(window)->model()->rowCount(), 5);
    QCOMPARE(statusTextOf(window),
             QStringLiteral("Loaded 2 products · Evaluating hardware profile..."));

    waitForOverlay(window);
    QCOMPARE(entryTableOf(window)->model()->rowCount(), 5);
}

void MainWindowHardwareTest::toolbarSummaryAndTooltip()
{
    MainWindow window;

    // Up to four values are listed in full.
    window.applyHardwareProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")},
                                 {QStringLiteral("CPUARCH"), QStringLiteral("R4400")},
                                 {QStringLiteral("GFXBOARD"), QStringLiteral("EXPRESS")},
                                 {QStringLiteral("MODE"), QStringLiteral("32bit")}});
    QCOMPARE(hardwareButtonOf(window)->text(),
             QStringLiteral("Hardware: IP22 / R4400 / EXPRESS / 32bit"));
    QCOMPARE(hardwareButtonOf(window)->toolTip(),
             QStringLiteral("CPUBOARD=IP22\nCPUARCH=R4400\nGFXBOARD=EXPRESS\nMODE=32bit"));

    // Beyond that the summary folds; the tooltip stays complete,
    // one pair per line, multi-valued attributes included.
    window.applyHardwareProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")},
                                 {QStringLiteral("CPUARCH"), QStringLiteral("MIPS2")},
                                 {QStringLiteral("CPUARCH"), QStringLiteral("R4000")},
                                 {QStringLiteral("GFXBOARD"), QStringLiteral("EXPRESS")},
                                 {QStringLiteral("MODE"), QStringLiteral("32bit")}});
    QCOMPARE(hardwareButtonOf(window)->text(),
             QStringLiteral("Hardware: IP22 / MIPS2 / R4000 / +2"));
    QCOMPARE(hardwareButtonOf(window)->toolTip(),
             QStringLiteral("CPUBOARD=IP22\nCPUARCH=MIPS2\nCPUARCH=R4000\nGFXBOARD=EXPRESS\n"
                            "MODE=32bit"));

    window.applyHardwareProfile({});
    QCOMPARE(hardwareButtonOf(window)->text(), QStringLiteral("Hardware: Off"));
}

void MainWindowHardwareTest::clearingProfileDisablesOverlay()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    window.applyHardwareProfile(ip22Profile());
    waitForOverlay(window);

    QSignalSpy selectionSpy(&window, &MainWindow::selectionRequested);
    window.applyHardwareProfile({});
    // An empty profile disables the selection without a backend
    // request.
    QCOMPARE(selectionSpy.count(), 0);
    QVERIFY(entryTableOf(window)->isColumnHidden(EntryTableModel::StatusColumn));
    QCOMPARE(hardwareButtonOf(window)->text(), QStringLiteral("Hardware: Off"));
    QCOMPARE(statusTextOf(window), QStringLiteral("Loaded 2 products"));
}

void MainWindowHardwareTest::staleSelectionResponseIsDropped()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    window.applyHardwareProfile(ip22Profile());
    waitForOverlay(window);
    QCOMPARE(entryTableOf(window)->model()->rowCount(), 0);

    // A response carrying an unknown request id belongs to a profile
    // the user has already replaced: it must not touch the overlay.
    SelectionSnapshot bogus;
    bogus.selected = {{1, 0}};
    const bool invoked = QMetaObject::invokeMethod(
        &window, "onSelectionReady", Qt::DirectConnection, Q_ARG(quint64, quint64(999999)),
        Q_ARG(SelectionSnapshot, bogus));
    QVERIFY(invoked);

    QCOMPARE(entryTableOf(window)->model()->rowCount(), 0);
    QCOMPARE(statusTextOf(window),
             QStringLiteral("Loaded 2 products · 6 selected records · 1 conflict groups"));
}

void MainWindowHardwareTest::profileChangeLeavesSearchResultsUntouched()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());

    QLineEdit *search = window.findChild<QLineEdit *>(QStringLiteral("searchEdit"));
    search->setText(QStringLiteral("libGL"));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);

    QSignalSpy searchSpy(&window, &MainWindow::searchEntriesRequested);
    QSignalSpy entriesSpy(&window, &MainWindow::entriesRequested);

    window.applyHardwareProfile(ip22Profile());
    waitForOverlay(window);

    // The search result set is identical; only the Status overlay
    // landed on it.
    QCOMPARE(searchSpy.count(), 0);
    QCOMPARE(entriesSpy.count(), 0);
    QCOMPARE(entryTableOf(window)->model()->rowCount(), 1);
    QCOMPARE(entryTableOf(window)
                 ->model()
                 ->index(0, EntryTableModel::PathColumn)
                 .data()
                 .toString(),
             QStringLiteral("usr/lib/libGL.so"));
    QCOMPARE(entryTableOf(window)
                 ->model()
                 ->index(0, EntryTableModel::StatusColumn)
                 .data()
                 .toString(),
             QStringLiteral("Selected"));
    QCOMPARE(labelOf(window, QStringLiteral("scopeLabel"))->text(),
             QStringLiteral("Search: libGL"));

    window.applyHardwareProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP26")}});
    QTRY_COMPARE(entryTableOf(window)
                     ->model()
                     ->index(0, EntryTableModel::StatusColumn)
                     .data()
                     .toString(),
                 QStringLiteral("Not selected"));
    QCOMPARE(entryTableOf(window)->model()->rowCount(), 1);
    QCOMPARE(searchSpy.count(), 0);
    QCOMPARE(entriesSpy.count(), 0);
}

void MainWindowHardwareTest::profileChangeLeavesHierarchyScopeUntouched()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());

    treeOf(window)->setCurrentIndex(productIndex(treeOf(window)->model(), QStringLiteral("alpha")));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 5);
    const QString firstPath =
        entryTableOf(window)->model()->index(0, EntryTableModel::PathColumn).data().toString();

    QSignalSpy entriesSpy(&window, &MainWindow::entriesRequested);
    window.applyHardwareProfile(ip22Profile());
    waitForOverlay(window);

    // Still the same five rows in the same order.
    QCOMPARE(entriesSpy.count(), 0);
    QCOMPARE(entryTableOf(window)->model()->rowCount(), 5);
    QCOMPARE(entryTableOf(window)->model()->index(0, EntryTableModel::PathColumn).data().toString(),
             firstPath);

    // Clicking another scope reloads entries but never re-selects:
    // the current overlay simply applies to the new rows.
    QSignalSpy selectionSpy(&window, &MainWindow::selectionRequested);
    treeOf(window)->setCurrentIndex(productIndex(treeOf(window)->model(), QStringLiteral("beta")));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);
    QCOMPARE(selectionSpy.count(), 0);
    QCOMPARE(entryTableOf(window)->model()->index(0, EntryTableModel::StatusColumn).data().toString(),
             QStringLiteral("Selected"));
}

void MainWindowHardwareTest::switchingProfileOnlyChangesStatuses()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());

    treeOf(window)->setCurrentIndex(productIndex(treeOf(window)->model(), QStringLiteral("alpha")));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 5);

    window.applyHardwareProfile(ip22Profile());
    waitForOverlay(window);
    QCOMPARE(entryTableOf(window)->model()->index(1, EntryTableModel::StatusColumn).data().toString(),
             QStringLiteral("Selected"));

    // IP22 -> IP26: no reload, just a new overlay.
    QSignalSpy entriesSpy(&window, &MainWindow::entriesRequested);
    window.applyHardwareProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP26")}});
    QTRY_COMPARE(entryTableOf(window)
                     ->model()
                     ->index(1, EntryTableModel::StatusColumn)
                     .data()
                     .toString(),
                 QStringLiteral("Not selected"));
    QCOMPARE(entriesSpy.count(), 0);
    QCOMPARE(entryTableOf(window)->model()->rowCount(), 5);
    QCOMPARE(statusTextOf(window),
             QStringLiteral("Loaded 2 products · 5 selected records · 1 conflict groups"));
}

void MainWindowHardwareTest::failedOpenKeepsTheWholeHardwareState()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    window.applyHardwareProfile(ip22Profile());
    waitForOverlay(window);
    window.show();
    QVERIFY(QTest::qWaitForWindowExposed(&window));

    // Opening a broken distribution pops the error box; accept it
    // from a watcher so the test does not block on the modal dialog.
    bool dialogSeen = false;
    QTimer closer;
    closer.setInterval(50);
    connect(&closer, &QTimer::timeout, [&dialogSeen] {
        const QWidgetList topLevels = QApplication::topLevelWidgets();
        for (QWidget *topLevel : topLevels) {
            if (auto *box = qobject_cast<QMessageBox *>(topLevel)) {
                dialogSeen = true;
                box->accept();
            }
        }
    });
    closer.start();

    window.openDistributionRequested(QStringLiteral("/definitely/missing/distribution"));
    QTRY_VERIFY(dialogSeen);
    closer.stop();

    // Everything hardware survived the failed open: profile, button,
    // overlay, statuses, status bar counts — and the button is
    // unlocked again.
    QToolButton *button = hardwareButtonOf(window);
    QVERIFY(button->isEnabled());
    QCOMPARE(button->text(), QStringLiteral("Hardware: IP22"));
    QVERIFY(!entryTableOf(window)->isColumnHidden(EntryTableModel::StatusColumn));
    QCOMPARE(statusTextOf(window),
             QStringLiteral("Loaded 2 products · 6 selected records · 1 conflict groups"));
}

void MainWindowHardwareTest::successfulReopenKeepsProfileAndReselects()
{
    QTemporaryDir dirA;
    QVERIFY(writeHardwareDist(dirA));
    QTemporaryDir dirB;
    QVERIFY(writeGammaDist(dirB));

    MainWindow window;
    loadDistribution(window, dirA.path());
    window.applyHardwareProfile(ip22Profile());
    waitForOverlay(window);
    QCOMPARE(statusTextOf(window),
             QStringLiteral("Loaded 2 products · 6 selected records · 1 conflict groups"));

    // The accepted candidate kills the old selection and
    // suggestions; the stored profile is re-evaluated after commit.
    // (Wait for B's own tree shape: "any rows" and "enabled" are
    // both already true while A is still up.)
    window.openDistributionRequested(dirB.path());
    QTRY_COMPARE(treeOf(window)->model()->rowCount(), 1);
    QTRY_VERIFY(treeOf(window)->isEnabled());
    waitForOverlay(window);
    QCOMPARE(statusTextOf(window),
             QStringLiteral("Loaded 1 products · 1 selected records · 0 conflict groups"));
    QCOMPARE(hardwareButtonOf(window)->text(), QStringLiteral("Hardware: IP22"));

    // The new distribution's rows carry the fresh selection.
    treeOf(window)->setCurrentIndex(productIndex(treeOf(window)->model(), QStringLiteral("gamma")));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);
    QCOMPARE(entryTableOf(window)->model()->index(0, EntryTableModel::StatusColumn).data().toString(),
             QStringLiteral("Selected"));
}

void MainWindowHardwareTest::hardwareButtonLocksDuringOpen()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    MainWindow window;
    QToolButton *button = hardwareButtonOf(window);
    QVERIFY(button->isEnabled());

    // The lock lands with the request itself, ahead of any worker
    // roundtrip.
    window.openDistributionRequested(dir.path());
    QVERIFY(!button->isEnabled());

    // Commit unlocks it again.
    QTRY_VERIFY(button->isEnabled());
    QVERIFY(treeOf(window)->isEnabled());
}

void MainWindowHardwareTest::candidatesRefreshAfterCommit()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    MainWindow window;
    window.applyHardwareProfile({{QStringLiteral("CPUBOARD"), QString()}});
    loadDistribution(window, dir.path());
    window.show();
    QVERIFY(QTest::qWaitForWindowExposed(&window));

    // The dialog's value suggestions come from the committed
    // distribution's own MACH expressions.
    QTRY_VERIFY(withHardwareDialog(window, [](HardwareProfileDialog *dialog) {
        auto *view = dialog->findChild<QTableView *>(QStringLiteral("profileTable"));
        if (view == nullptr || view->model()->rowCount() < 1) {
            return false;
        }
        view->setCurrentIndex(view->model()->index(0, 1));
        view->edit(view->model()->index(0, 1));
        auto *combo = view->findChild<QComboBox *>();
        if (combo == nullptr) {
            return false;
        }
        const bool found = combo->findText(QStringLiteral("IP22")) >= 0;
        view->setFocus(Qt::OtherFocusReason);
        return found;
    }));
}

void MainWindowHardwareTest::staleCandidatesResponseIsDropped()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    MainWindow window;
    window.applyHardwareProfile({{QStringLiteral("CPUBOARD"), QString()}});
    loadDistribution(window, dir.path());
    window.show();
    QVERIFY(QTest::qWaitForWindowExposed(&window));

    // Wait for the real candidates, then inject a stale response:
    // the suggestions must stay the real ones.
    QTRY_VERIFY(withHardwareDialog(window, [](HardwareProfileDialog *dialog) {
        auto *view = dialog->findChild<QTableView *>(QStringLiteral("profileTable"));
        view->setCurrentIndex(view->model()->index(0, 1));
        view->edit(view->model()->index(0, 1));
        auto *combo = view->findChild<QComboBox *>();
        if (combo == nullptr) {
            return false;
        }
        const bool found = combo->findText(QStringLiteral("IP22")) >= 0;
        view->setFocus(Qt::OtherFocusReason);
        return found;
    }));

    HardwareCandidatesSnapshot bogus = {
        {QStringLiteral("CPUBOARD"), {QStringLiteral("BOGUSBOARD")}}};
    const bool invoked = QMetaObject::invokeMethod(
        &window, "onHardwareCandidatesReady", Qt::DirectConnection,
        Q_ARG(quint64, quint64(999999)), Q_ARG(HardwareCandidatesSnapshot, bogus));
    QVERIFY(invoked);

    QVERIFY(withHardwareDialog(window, [](HardwareProfileDialog *dialog) {
        auto *view = dialog->findChild<QTableView *>(QStringLiteral("profileTable"));
        view->setCurrentIndex(view->model()->index(0, 1));
        view->edit(view->model()->index(0, 1));
        auto *combo = view->findChild<QComboBox *>();
        if (combo == nullptr) {
            return false;
        }
        const bool realKept = combo->findText(QStringLiteral("IP22")) >= 0;
        const bool bogusDropped = combo->findText(QStringLiteral("BOGUSBOARD")) < 0;
        view->setFocus(Qt::OtherFocusReason);
        return realKept && bogusDropped;
    }));
}

void MainWindowHardwareTest::selectionErrorKeepsProfileAndFiles()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());

    treeOf(window)->setCurrentIndex(productIndex(treeOf(window)->model(), QStringLiteral("alpha")));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 5);

    // A working selection first...
    window.applyHardwareProfile(ip22Profile());
    waitForOverlay(window);

    // ...then a profile whose blank attribute the bridge rejects: the
    // full failure path, end to end.
    window.applyHardwareProfile({{QString(), QStringLiteral("IP22")}});
    QTRY_VERIFY(
        statusTextOf(window).contains(QStringLiteral("Hardware selection unavailable")));

    // The profile stays applied and the files stay; the overlay is
    // gone and the status bar carries the error.
    QCOMPARE(statusTextOf(window),
             QStringLiteral("Loaded 2 products · Hardware selection unavailable: hardware "
                            "attribute name is empty"));
    QCOMPARE(hardwareButtonOf(window)->toolTip(), QStringLiteral("=IP22"));
    QVERIFY(entryTableOf(window)->isColumnHidden(EntryTableModel::StatusColumn));
    QCOMPARE(entryTableOf(window)->model()->rowCount(), 5);

    // An ordinary successful render does not pretend the selection
    // error away.
    treeOf(window)->setCurrentIndex(productIndex(treeOf(window)->model(), QStringLiteral("beta")));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);
    QVERIFY(statusTextOf(window).contains(QStringLiteral("Hardware selection unavailable")));
}

void MainWindowHardwareTest::emptyValueProfileSelectsHeadlessRecord()
{
    QTemporaryDir dir;
    QVERIFY(writeEmptyValueDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());

    treeOf(window)->setCurrentIndex(productIndex(treeOf(window)->model(), QStringLiteral("gfx")));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 2);

    // The empty value survives the whole path — profile, bridge,
    // core selection — and selects the `GFXBOARD=` record.
    window.applyHardwareProfile({{QStringLiteral("GFXBOARD"), QString()}});
    waitForOverlay(window);

    QCOMPARE(hardwareButtonOf(window)->text(), QStringLiteral("Hardware: (empty)"));
    QCOMPARE(hardwareButtonOf(window)->toolTip(), QStringLiteral("GFXBOARD="));
    QCOMPARE(statusTextOf(window),
             QStringLiteral("Loaded 1 products · 2 selected records · 0 conflict groups"));

    const QAbstractItemModel *model = entryTableOf(window)->model();
    QCOMPARE(model->index(0, EntryTableModel::PathColumn).data().toString(),
             QStringLiteral("usr/bin/any"));
    QCOMPARE(model->index(0, EntryTableModel::StatusColumn).data().toString(),
             QStringLiteral("Selected"));
    QCOMPARE(model->index(1, EntryTableModel::PathColumn).data().toString(),
             QStringLiteral("usr/bin/headless"));
    QCOMPARE(model->index(1, EntryTableModel::StatusColumn).data().toString(),
             QStringLiteral("Selected"));

    // A non-empty board rejects the headless record; the mach-less
    // one stays selected. The selected-record count proves the new
    // selection has landed before the statuses are checked.
    window.applyHardwareProfile({{QStringLiteral("GFXBOARD"), QStringLiteral("EXPRESS")}});
    QTRY_COMPARE(statusTextOf(window),
                 QStringLiteral("Loaded 1 products · 1 selected records · 0 conflict groups"));
    QCOMPARE(model->index(0, EntryTableModel::StatusColumn).data().toString(),
             QStringLiteral("Selected"));
    QCOMPARE(model->index(1, EntryTableModel::StatusColumn).data().toString(),
             QStringLiteral("Not selected"));
    QCOMPARE(model->rowCount(), 2);
}

QTEST_MAIN(MainWindowHardwareTest)

#include "MainWindowHardwareTest.moc"
