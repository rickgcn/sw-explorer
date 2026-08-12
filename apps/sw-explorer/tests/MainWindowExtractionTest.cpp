#include "MainWindow.h"

#include <QAbstractItemModel>
#include <QAction>
#include <QApplication>
#include <QCheckBox>
#include <QFile>
#include <QLabel>
#include <QLineEdit>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QRadioButton>
#include <QSignalSpy>
#include <QTableView>
#include <QTemporaryDir>
#include <QTest>
#include <QTimer>
#include <QToolButton>
#include <QTreeView>

#include "EntryBrowserWidget.h"
#include "ExtractionDialog.h"
#include "models/EntryTableModel.h"

// Drives the real MainWindow against the real Rust backend (through
// BackendWorker on its thread): the Extract action's availability, the
// extraction scope of the current Files view, the hardware profile in
// the request, and the extraction request-family lifecycle.
//
// Distributions: `alpha`/`beta` for scope and availability tests
// (usr/bin/Xsgi in both products, so a search shows two rows), and
// `ext`/`miss` for extraction flow tests: `ext` has a real image
// archive with four extractable entries, `miss` has a payload-bearing
// record without an archive (a runtime read failure). Object ids in
// the flow distribution: 1 = ext, 2 = ext.sw, 3 = unix, 4 = miss,
// 5 = miss.sw, 6 = unix.
class MainWindowExtractionTest : public QObject
{
    Q_OBJECT

private slots:
    void initTestCase();
    void extractActionIsDisabledAtStartup();
    void extractActionNeedsVisibleFilesRows();
    void searchLoadingDisablesAndResultsEnable();
    void hardwareSelectionPendingOrErrorDisables();
    void currentFilesRequestCarriesDisplayedKeys();
    void searchResultsScopeUsesDisplayedRows();
    void selectedEntryScopeCarriesOneKey();
    void inactiveProfileSendsEmptyHardwareList();
    void activeProfileIsCopiedVerbatim();
    void dialogPreflightExtractFlow();
    void dialogPreflightRefusalKeepsOptions();
    void staleExtractionResponsesAreDropped();
    void runningExtractionLocksAndFinishRestores();
    void plannerFailureRestoresControls();
    void runtimeFailuresStillRestoreControls();
};

namespace {

bool writeAlphaBetaDist(QTemporaryDir &dir)
{
    QFile alphaIdb(dir.filePath(QStringLiteral("alpha.idb")));
    if (!alphaIdb.open(QIODevice::WriteOnly)) {
        return false;
    }
    alphaIdb.write(
        "f 0755 root sys usr/bin/Xsgi src/Xsgi alpha.sw.unix sum(1) size(100) cmpsize(60)\n"
        "f 0644 root sys usr/lib/libGL.so src/libGL alpha.sw.gfx sum(2) size(200) cmpsize(0) mach(CPUBOARD=IP22)\n"
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

void appendArchiveRecord(QFile &archive, const QByteArray &name, const QByteArray &payload)
{
    const quint16 length = static_cast<quint16>(name.size());
    const char header[2] = {static_cast<char>(length >> 8), static_cast<char>(length & 0xff)};
    archive.write(header, 2);
    archive.write(name);
    archive.write(payload);
}

bool writeFlowDist(QTemporaryDir &dir)
{
    QFile idb(dir.filePath(QStringLiteral("ext.idb")));
    if (!idb.open(QIODevice::WriteOnly)) {
        return false;
    }
    idb.write(
        "d 0755 root sys bin src ext.sw.unix\n"
        "f 0644 root sys hello.txt src/hello.txt ext.sw.unix sum(1) size(5) cmpsize(0)\n"
        "f 0755 root sys bin/tool src/tool ext.sw.unix sum(2) size(4) cmpsize(0)\n"
        "f 0644 root sys empty.txt src/empty ext.sw.unix sum(3) size(0)\n");
    idb.close();

    QFile archive(dir.filePath(QStringLiteral("ext.sw")));
    if (!archive.open(QIODevice::WriteOnly)) {
        return false;
    }
    archive.write("im001V999P00\0", 13);
    appendArchiveRecord(archive, "hello.txt", "hello");
    appendArchiveRecord(archive, "bin/tool", "tool");
    archive.close();

    // A payload-bearing record without an image archive: planning is
    // fine, the read fails at runtime.
    QFile missIdb(dir.filePath(QStringLiteral("miss.idb")));
    if (!missIdb.open(QIODevice::WriteOnly)) {
        return false;
    }
    missIdb.write("f 0644 root sys bad.txt src/bad miss.sw.unix sum(1) size(3) cmpsize(0)\n");
    return true;
}

QAction *extractActionOf(MainWindow &window)
{
    return window.findChild<QAction *>(QStringLiteral("extractAction"));
}

QAction *openActionOf(MainWindow &window)
{
    return window.findChild<QAction *>(QStringLiteral("openAction"));
}

QTreeView *treeOf(MainWindow &window)
{
    return window.findChild<QTreeView *>();
}

QTableView *entryTableOf(MainWindow &window)
{
    return window.findChild<QTableView *>(QStringLiteral("entryTable"));
}

QLineEdit *searchEditOf(MainWindow &window)
{
    return window.findChild<QLineEdit *>(QStringLiteral("searchEdit"));
}

QToolButton *hardwareButtonOf(MainWindow &window)
{
    return window.findChild<QToolButton *>(QStringLiteral("hardwareButton"));
}

void loadDistribution(MainWindow &window, const QString &path)
{
    window.openDistribution(path);
    QTRY_VERIFY(treeOf(window)->model()->rowCount() > 0);
    QTRY_VERIFY(treeOf(window)->isEnabled());
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

// Selects a product in the tree and waits for its Files rows.
void selectProduct(MainWindow &window, const QString &name)
{
    treeOf(window)->setCurrentIndex(productIndex(treeOf(window)->model(), name));
    QTRY_VERIFY(entryTableOf(window)->model()->rowCount() > 0);
}

// Runs `drive` on the modal extraction dialog tick by tick until the
// dialog closes; the driver's own state decides what each tick does.
// Returns whether the dialog showed up at all.
template <typename F>
bool withExtractionDialog(MainWindow &window, F drive)
{
    bool seen = false;
    QTimer watcher;
    watcher.setInterval(50);
    QObject::connect(&watcher, &QTimer::timeout, [&] {
        auto *dialog = qobject_cast<ExtractionDialog *>(QApplication::activeModalWidget());
        if (dialog != nullptr) {
            seen = true;
            drive(dialog);
        }
    });
    watcher.start();
    extractActionOf(window)->trigger();
    watcher.stop();
    return seen;
}

} // namespace

void MainWindowExtractionTest::initTestCase()
{
    qRegisterMetaType<HierarchyKind>("HierarchyKind");
    qRegisterMetaType<EntryListSnapshot>("EntryListSnapshot");
    qRegisterMetaType<HardwareProfileSnapshot>("HardwareProfileSnapshot");
    qRegisterMetaType<HardwareCandidatesSnapshot>("HardwareCandidatesSnapshot");
    qRegisterMetaType<SelectionSnapshot>("SelectionSnapshot");
    qRegisterMetaType<ExtractionRequestSnapshot>("ExtractionRequestSnapshot");
    qRegisterMetaType<ExtractionPlanSnapshot>("ExtractionPlanSnapshot");
    qRegisterMetaType<ExtractionReportSnapshot>("ExtractionReportSnapshot");
}

void MainWindowExtractionTest::extractActionIsDisabledAtStartup()
{
    MainWindow window;
    QVERIFY(extractActionOf(window) != nullptr);
    QVERIFY(!extractActionOf(window)->isEnabled());
}

void MainWindowExtractionTest::extractActionNeedsVisibleFilesRows()
{
    QTemporaryDir dir;
    QVERIFY(writeAlphaBetaDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    // A distribution but no visible Files rows: nothing to extract.
    QVERIFY(!extractActionOf(window)->isEnabled());

    selectProduct(window, QStringLiteral("alpha"));
    QTRY_VERIFY(extractActionOf(window)->isEnabled());
}

void MainWindowExtractionTest::searchLoadingDisablesAndResultsEnable()
{
    QTemporaryDir dir;
    QVERIFY(writeAlphaBetaDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    selectProduct(window, QStringLiteral("alpha"));
    QTRY_VERIFY(extractActionOf(window)->isEnabled());

    // While the search is loading, the Files view is not an extraction
    // scope.
    searchEditOf(window)->setText(QStringLiteral("Xsgi"));
    QVERIFY(!extractActionOf(window)->isEnabled());

    // The finished search result list is one.
    QTRY_VERIFY(extractActionOf(window)->isEnabled());
    QCOMPARE(entryTableOf(window)->model()->rowCount(), 2);
}

void MainWindowExtractionTest::hardwareSelectionPendingOrErrorDisables()
{
    QTemporaryDir dir;
    QVERIFY(writeAlphaBetaDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    selectProduct(window, QStringLiteral("alpha"));
    QTRY_VERIFY(extractActionOf(window)->isEnabled());

    // A profile whose selection is still being computed.
    window.applyHardwareProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});
    QVERIFY(!extractActionOf(window)->isEnabled());
    QTRY_VERIFY(extractActionOf(window)->isEnabled());

    // A profile the backend rejects: the selection state is Error and
    // extraction stays unavailable too.
    window.applyHardwareProfile({{QString(), QStringLiteral("IP22")}});
    QVERIFY(!extractActionOf(window)->isEnabled());
    QTest::qWait(300);
    QVERIFY(!extractActionOf(window)->isEnabled());

    // Clearing the profile re-enables.
    window.applyHardwareProfile({});
    QTRY_VERIFY(extractActionOf(window)->isEnabled());
}

void MainWindowExtractionTest::currentFilesRequestCarriesDisplayedKeys()
{
    QTemporaryDir dir;
    QVERIFY(writeFlowDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    selectProduct(window, QStringLiteral("ext"));
    QTRY_VERIFY(extractActionOf(window)->isEnabled());

    QSignalSpy requestSpy(&window, &MainWindow::planExtractionRequested);
    int step = 0;
    const bool seen = withExtractionDialog(window, [&](ExtractionDialog *dialog) {
        if (step == 0) {
            dialog->findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
                ->setText(dir.filePath(QStringLiteral("out")));
            dialog->findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
            step = 1;
        } else if (dialog->state() == ExtractionDialog::State::PlanReady) {
            dialog->reject();
            step = 2;
        }
    });
    QVERIFY(seen);
    QCOMPARE(requestSpy.count(), 1);
    // The scope is exactly the rows on screen, never a re-query.
    const auto request = qvariant_cast<ExtractionRequestSnapshot>(requestSpy.first().at(1));
    QCOMPARE(request.entries, (QList<EntryKey>{{1, 0}, {1, 1}, {1, 2}, {1, 3}}));
    QCOMPARE(request.outputDir, dir.filePath(QStringLiteral("out")));
    QVERIFY(request.hardware.isEmpty());
    QVERIFY(!request.allowOverwrite);
}

void MainWindowExtractionTest::searchResultsScopeUsesDisplayedRows()
{
    QTemporaryDir dir;
    QVERIFY(writeAlphaBetaDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    searchEditOf(window)->setText(QStringLiteral("Xsgi"));
    QTRY_VERIFY(extractActionOf(window)->isEnabled());
    QCOMPARE(entryTableOf(window)->model()->rowCount(), 2);

    QSignalSpy requestSpy(&window, &MainWindow::planExtractionRequested);
    int step = 0;
    const bool seen = withExtractionDialog(window, [&](ExtractionDialog *dialog) {
        if (step == 0) {
            dialog->findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
                ->setText(dir.filePath(QStringLiteral("out")));
            dialog->findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
            step = 1;
        } else if (dialog->state() != ExtractionDialog::State::Checking) {
            // Plan or refusal: both fine — only the request matters.
            dialog->reject();
            step = 2;
        }
    });
    QVERIFY(seen);
    QCOMPARE(requestSpy.count(), 1);
    // The search rows are the scope — both Xsgi records, not a fresh
    // query for the search text.
    const auto request = qvariant_cast<ExtractionRequestSnapshot>(requestSpy.first().at(1));
    QCOMPARE(request.entries, (QList<EntryKey>{{1, 0}, {5, 0}}));
}

void MainWindowExtractionTest::selectedEntryScopeCarriesOneKey()
{
    QTemporaryDir dir;
    QVERIFY(writeFlowDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    selectProduct(window, QStringLiteral("ext"));
    entryTableOf(window)->setCurrentIndex(entryTableOf(window)->model()->index(1, 0));
    QTRY_VERIFY(extractActionOf(window)->isEnabled());

    QSignalSpy requestSpy(&window, &MainWindow::planExtractionRequested);
    int step = 0;
    const bool seen = withExtractionDialog(window, [&](ExtractionDialog *dialog) {
        if (step == 0) {
            auto *selected =
                dialog->findChild<QRadioButton *>(QStringLiteral("selectedEntryRadio"));
            QVERIFY(selected->isEnabled());
            selected->setChecked(true);
            dialog->findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
                ->setText(dir.filePath(QStringLiteral("out")));
            dialog->findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
            step = 1;
        } else if (dialog->state() != ExtractionDialog::State::Checking) {
            dialog->reject();
            step = 2;
        }
    });
    QVERIFY(seen);
    QCOMPARE(requestSpy.count(), 1);
    const auto request = qvariant_cast<ExtractionRequestSnapshot>(requestSpy.first().at(1));
    QCOMPARE(request.entries, (QList<EntryKey>{{1, 1}}));
}

void MainWindowExtractionTest::inactiveProfileSendsEmptyHardwareList()
{
    QTemporaryDir dir;
    QVERIFY(writeFlowDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    selectProduct(window, QStringLiteral("ext"));
    QTRY_VERIFY(extractActionOf(window)->isEnabled());

    QSignalSpy requestSpy(&window, &MainWindow::planExtractionRequested);
    int step = 0;
    const bool seen = withExtractionDialog(window, [&](ExtractionDialog *dialog) {
        if (step == 0) {
            dialog->findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
                ->setText(dir.filePath(QStringLiteral("out")));
            dialog->findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
            step = 1;
        } else if (dialog->state() != ExtractionDialog::State::Checking) {
            dialog->reject();
            step = 2;
        }
    });
    QVERIFY(seen);
    QCOMPARE(requestSpy.count(), 1);
    const auto request = qvariant_cast<ExtractionRequestSnapshot>(requestSpy.first().at(1));
    QVERIFY(request.hardware.isEmpty());
}

void MainWindowExtractionTest::activeProfileIsCopiedVerbatim()
{
    QTemporaryDir dir;
    QVERIFY(writeAlphaBetaDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    selectProduct(window, QStringLiteral("alpha"));
    window.applyHardwareProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")},
                                 {QStringLiteral("GFXBOARD"), QString()}});
    QTRY_VERIFY(extractActionOf(window)->isEnabled());

    QSignalSpy requestSpy(&window, &MainWindow::planExtractionRequested);
    int step = 0;
    const bool seen = withExtractionDialog(window, [&](ExtractionDialog *dialog) {
        if (step == 0) {
            dialog->findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
                ->setText(dir.filePath(QStringLiteral("out")));
            dialog->findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
            step = 1;
        } else if (dialog->state() != ExtractionDialog::State::Checking) {
            dialog->reject();
            step = 2;
        }
    });
    QVERIFY(seen);
    QCOMPARE(requestSpy.count(), 1);
    const auto request = qvariant_cast<ExtractionRequestSnapshot>(requestSpy.first().at(1));
    // The profile is the only hardware input: the on-screen selection
    // overlay never becomes the extraction authority.
    QCOMPARE(request.hardware.size(), 2);
    QCOMPARE(request.hardware.at(0).attribute, QStringLiteral("CPUBOARD"));
    QCOMPARE(request.hardware.at(0).value, QStringLiteral("IP22"));
    QCOMPARE(request.hardware.at(1).attribute, QStringLiteral("GFXBOARD"));
    // The empty value survives verbatim.
    QCOMPARE(request.hardware.at(1).value, QString());
}

void MainWindowExtractionTest::dialogPreflightExtractFlow()
{
    QTemporaryDir dir;
    QVERIFY(writeFlowDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    selectProduct(window, QStringLiteral("ext"));
    QTRY_VERIFY(extractActionOf(window)->isEnabled());

    int step = 0;
    bool sawPlan = false;
    bool sawResult = false;
    const bool seen = withExtractionDialog(window, [&](ExtractionDialog *dialog) {
        switch (step) {
        case 0:
            dialog->findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
                ->setText(dir.filePath(QStringLiteral("out")));
            dialog->findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
            step = 1;
            break;
        case 1:
            if (dialog->state() == ExtractionDialog::State::PlanReady) {
                sawPlan = true;
                dialog->findChild<QPushButton *>(QStringLiteral("extractButton"))->click();
                step = 2;
            }
            break;
        case 2:
            if (dialog->state() == ExtractionDialog::State::Done) {
                sawResult = true;
                const QString body =
                    dialog->findChild<QPlainTextEdit *>(QStringLiteral("resultBodyEdit"))
                        ->toPlainText();
                QVERIFY(body.contains(QStringLiteral("Extracted:    4")));
                QVERIFY(body.contains(QStringLiteral("Failed:       0")));
                dialog->findChild<QPushButton *>(QStringLiteral("resultCloseButton"))->click();
                step = 3;
            }
            break;
        default:
            break;
        }
    });
    QVERIFY(seen);
    QVERIFY(sawPlan);
    QVERIFY(sawResult);

    QFile extracted(dir.filePath(QStringLiteral("out/hello.txt")));
    QVERIFY(extracted.open(QIODevice::ReadOnly));
    QCOMPARE(extracted.readAll(), QByteArray("hello"));
    QVERIFY(QFile::exists(dir.filePath(QStringLiteral("out/bin/tool"))));
    // The extraction finished: interaction is restored.
    QVERIFY(openActionOf(window)->isEnabled());
    QVERIFY(treeOf(window)->isEnabled());
}

void MainWindowExtractionTest::dialogPreflightRefusalKeepsOptions()
{
    QTemporaryDir dir;
    QVERIFY(writeAlphaBetaDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    selectProduct(window, QStringLiteral("alpha"));
    QTRY_VERIFY(extractActionOf(window)->isEnabled());

    int step = 0;
    bool sawRefusal = false;
    const bool seen = withExtractionDialog(window, [&](ExtractionDialog *dialog) {
        switch (step) {
        case 0:
            dialog->findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
                ->setText(dir.filePath(QStringLiteral("out")));
            dialog->findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
            step = 1;
            break;
        case 1:
            if (dialog->state() == ExtractionDialog::State::Failed) {
                sawRefusal = true;
                // The usr/share/conf duplicates are ambiguous without
                // a profile.
                QVERIFY(dialog->findChild<QPlainTextEdit *>(
                                     QStringLiteral("failureMessageEdit"))
                            ->toPlainText()
                            .contains(QStringLiteral("ambiguous")));
                dialog->findChild<QPushButton *>(QStringLiteral("failureBackButton"))->click();
                step = 2;
            }
            break;
        case 2:
            // The options survived the refusal.
            QCOMPARE(dialog->findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))->text(),
                     dir.filePath(QStringLiteral("out")));
            dialog->reject();
            step = 3;
            break;
        default:
            break;
        }
    });
    QVERIFY(seen);
    QVERIFY(sawRefusal);
    // A preflight refusal never starts an extraction.
    QVERIFY(!QFile::exists(dir.filePath(QStringLiteral("out/hello.txt"))));
}

void MainWindowExtractionTest::staleExtractionResponsesAreDropped()
{
    QTemporaryDir dir;
    QVERIFY(writeFlowDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    selectProduct(window, QStringLiteral("ext"));
    QTRY_VERIFY(extractActionOf(window)->isEnabled());

    // Start an extraction whose request is invalid (empty output
    // directory): the worker's failure for request 1 is queued.
    ExtractionRequestSnapshot request;
    request.entries = {{1, 1}};
    QVERIFY(QMetaObject::invokeMethod(&window,
                                      "onExtractionRequested",
                                      Qt::DirectConnection,
                                      Q_ARG(ExtractionRequestSnapshot, request)));
    QVERIFY(!openActionOf(window)->isEnabled());

    // The user moved on before the response landed: request 1 is
    // stale; its late failure must not unlock anything.
    QVERIFY(QMetaObject::invokeMethod(&window, "onExtractionInvalidated", Qt::DirectConnection));
    QTest::qWait(300);
    QVERIFY(!openActionOf(window)->isEnabled());

    // Only the response of the active request resolves the state.
    QVERIFY(QMetaObject::invokeMethod(&window,
                                      "onExtractionFailed",
                                      Qt::DirectConnection,
                                      Q_ARG(quint64, 2),
                                      Q_ARG(QString, QStringLiteral("refused"))));
    QVERIFY(openActionOf(window)->isEnabled());
    QVERIFY(treeOf(window)->isEnabled());
}

void MainWindowExtractionTest::runningExtractionLocksAndFinishRestores()
{
    QTemporaryDir dir;
    QVERIFY(writeFlowDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    selectProduct(window, QStringLiteral("ext"));
    QTRY_VERIFY(extractActionOf(window)->isEnabled());

    ExtractionRequestSnapshot request;
    request.entries = {{1, 1}};
    request.outputDir = dir.filePath(QStringLiteral("out"));
    QVERIFY(QMetaObject::invokeMethod(&window,
                                      "onExtractionRequested",
                                      Qt::DirectConnection,
                                      Q_ARG(ExtractionRequestSnapshot, request)));

    // Before the queued completion lands: everything is locked.
    QVERIFY(!openActionOf(window)->isEnabled());
    QVERIFY(!treeOf(window)->isEnabled());
    QVERIFY(!searchEditOf(window)->isEnabled());
    QVERIFY(!hardwareButtonOf(window)->isEnabled());
    QVERIFY(!extractActionOf(window)->isEnabled());

    // The batch completes; every lock is released.
    QTRY_VERIFY(openActionOf(window)->isEnabled());
    QVERIFY(treeOf(window)->isEnabled());
    QVERIFY(searchEditOf(window)->isEnabled());
    QVERIFY(hardwareButtonOf(window)->isEnabled());
    QVERIFY(extractActionOf(window)->isEnabled());
    QVERIFY(QFile::exists(dir.filePath(QStringLiteral("out/hello.txt"))));
}

void MainWindowExtractionTest::plannerFailureRestoresControls()
{
    QTemporaryDir dir;
    QVERIFY(writeFlowDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    selectProduct(window, QStringLiteral("ext"));
    QTRY_VERIFY(extractActionOf(window)->isEnabled());

    // The target already exists and overwriting is off: the re-plan
    // inside the checked extraction refuses before any write.
    QDir().mkpath(dir.filePath(QStringLiteral("out")));
    QFile existing(dir.filePath(QStringLiteral("out/hello.txt")));
    QVERIFY(existing.open(QIODevice::WriteOnly));
    existing.write("KEEP");
    existing.close();

    ExtractionRequestSnapshot request;
    request.entries = {{1, 1}};
    request.outputDir = dir.filePath(QStringLiteral("out"));
    QVERIFY(QMetaObject::invokeMethod(&window,
                                      "onExtractionRequested",
                                      Qt::DirectConnection,
                                      Q_ARG(ExtractionRequestSnapshot, request)));
    QVERIFY(!openActionOf(window)->isEnabled());

    QTRY_VERIFY(openActionOf(window)->isEnabled());
    QVERIFY(extractActionOf(window)->isEnabled());
    QVERIFY(existing.open(QIODevice::ReadOnly));
    QCOMPARE(existing.readAll(), QByteArray("KEEP"));
}

void MainWindowExtractionTest::runtimeFailuresStillRestoreControls()
{
    QTemporaryDir dir;
    QVERIFY(writeFlowDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    selectProduct(window, QStringLiteral("ext"));
    QTRY_VERIFY(extractActionOf(window)->isEnabled());

    // hello.txt extracts; bad.txt fails at read time (no archive).
    // A report with failures is not a planner refusal.
    ExtractionRequestSnapshot request;
    request.entries = {{1, 1}, {4, 0}};
    request.outputDir = dir.filePath(QStringLiteral("out"));
    QVERIFY(QMetaObject::invokeMethod(&window,
                                      "onExtractionRequested",
                                      Qt::DirectConnection,
                                      Q_ARG(ExtractionRequestSnapshot, request)));
    QVERIFY(!openActionOf(window)->isEnabled());

    QTRY_VERIFY(openActionOf(window)->isEnabled());
    QVERIFY(extractActionOf(window)->isEnabled());
    QVERIFY(QFile::exists(dir.filePath(QStringLiteral("out/hello.txt"))));
    QVERIFY(!QFile::exists(dir.filePath(QStringLiteral("out/bad.txt"))));
}

QTEST_MAIN(MainWindowExtractionTest)

#include "MainWindowExtractionTest.moc"
