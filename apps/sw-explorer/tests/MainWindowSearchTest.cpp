#include "MainWindow.h"

#include <QAbstractItemModel>
#include <QFile>
#include <QKeySequence>
#include <QLabel>
#include <QLineEdit>
#include <QMessageBox>
#include <QScrollArea>
#include <QSignalSpy>
#include <QStackedWidget>
#include <QTableView>
#include <QTemporaryDir>
#include <QTest>
#include <QTimer>
#include <QToolBar>
#include <QTreeView>

#include "EntryBrowserWidget.h"
#include "InspectorWidget.h"

// Drives the real MainWindow against the real Rust backend (through
// BackendWorker on its thread) with a synthetic two-product
// distribution: the search box state machine, the debounce, the
// tree/search context switch and the stale-response handling.
//
// The distribution: `alpha` with usr/bin/Xsgi in alpha.sw.unix and
// usr/lib/libGL.so in alpha.sw.gfx, `beta` with a second
// usr/bin/Xsgi record in beta.sw.unix. Object ids: 1 = alpha,
// 2 = alpha.sw, 3 = unix, 4 = gfx, 5 = beta, 6 = beta.sw, 7 = unix.
class MainWindowSearchTest : public QObject
{
    Q_OBJECT

private slots:
    void initTestCase();
    void toolbarExposesSearchField();
    void findShortcutFocusesSearch();
    void searchWorksWithoutTreeSelection();
    void debounceCoalescesKeystrokes();
    void returnKeySearchesImmediately();
    void zeroResultsShowTheEmptyTable();
    void clearRestoresHierarchyScope();
    void differentRowClickExitsSearch();
    void sameRowClickExitsSearch();
    void staleEntriesResponseIsDropped();
    void searchResultClickShowsInspector();
    void queryChangeInvalidatesInspector();
    void failedOpenKeepsSearchState();
    void successfulReopenClearsSearchState();
};

namespace {

bool writeTwoProductDist(QTemporaryDir &dir)
{
    QFile alphaIdb(dir.filePath(QStringLiteral("alpha.idb")));
    if (!alphaIdb.open(QIODevice::WriteOnly)) {
        return false;
    }
    alphaIdb.write(
        "f 0755 root sys usr/bin/Xsgi src/Xsgi alpha.sw.unix sum(1) size(100) cmpsize(60)\n");
    alphaIdb.write(
        "f 0644 root sys usr/lib/libGL.so src/libGL alpha.sw.gfx sum(2) size(200) cmpsize(0)\n");
    alphaIdb.close();

    QFile betaIdb(dir.filePath(QStringLiteral("beta.idb")));
    if (!betaIdb.open(QIODevice::WriteOnly)) {
        return false;
    }
    betaIdb.write(
        "f 0755 root sys usr/bin/Xsgi src/Xsgi-beta beta.sw.unix sum(3) size(110) cmpsize(0)\n");
    return true;
}

// One product `gamma` with a single usr/bin/gamma record.
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

QLineEdit *searchEditOf(MainWindow &window)
{
    return window.findChild<QLineEdit *>(QStringLiteral("searchEdit"));
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

QStackedWidget *stackOf(QWidget *widget)
{
    return widget->findChild<QStackedWidget *>();
}

// Opens the distribution and waits until the candidate is committed:
// the tree carries the new hierarchy and interaction is unlocked.
void loadDistribution(MainWindow &window, const QString &path)
{
    window.openDistributionRequested(path);
    QTRY_VERIFY(treeOf(window)->model()->rowCount() > 0);
    QTRY_VERIFY(treeOf(window)->isEnabled());
    QTRY_VERIFY(searchEditOf(window)->isEnabled());
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

} // namespace

void MainWindowSearchTest::initTestCase()
{
    qRegisterMetaType<HierarchyKind>("HierarchyKind");
    qRegisterMetaType<EntryListSnapshot>("EntryListSnapshot");
}

void MainWindowSearchTest::toolbarExposesSearchField()
{
    MainWindow window;

    auto *toolbar = window.findChild<QToolBar *>(QStringLiteral("mainToolBar"));
    QVERIFY(toolbar != nullptr);
    QVERIFY(!toolbar->isMovable());

    QLineEdit *search = searchEditOf(window);
    QVERIFY(search != nullptr);
    QCOMPARE(search->placeholderText(), QStringLiteral("Search paths..."));
    QVERIFY(search->isClearButtonEnabled());
    // No distribution: nothing to search.
    QVERIFY(!search->isEnabled());

    // The toolbar reuses the one open action; no copy exists.
    int openActions = 0;
    for (QAction *action : toolbar->actions()) {
        if (action->text() == QStringLiteral("Open Distribution...")) {
            ++openActions;
        }
    }
    QCOMPARE(openActions, 1);

    // A committed distribution enables the box.
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));
    loadDistribution(window, dir.path());
    QVERIFY(search->isEnabled());
}

void MainWindowSearchTest::findShortcutFocusesSearch()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    window.show();
    QVERIFY(QTest::qWaitForWindowActive(&window));

    QLineEdit *search = searchEditOf(window);
    search->setText(QStringLiteral("libGL"));

    const QKeyCombination find = QKeySequence(QKeySequence::Find)[0];
    QTest::keyClick(&window, find.key(), find.keyboardModifiers());
    QCOMPARE(window.focusWidget(), search);
    QCOMPARE(search->selectedText(), QStringLiteral("libGL"));
}

void MainWindowSearchTest::searchWorksWithoutTreeSelection()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());

    // No tree node was ever selected: a global search only needs the
    // committed distribution. Xsgi lives in alpha and beta.
    QSignalSpy searchSpy(&window, &MainWindow::searchEntriesRequested);
    searchEditOf(window)->setText(QStringLiteral("Xsgi"));
    QTRY_COMPARE(searchSpy.count(), 1);
    QCOMPARE(searchSpy.first().at(1).toString(), QStringLiteral("Xsgi"));

    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 2);
    QCOMPARE(labelOf(window, QStringLiteral("scopeLabel"))->text(),
             QStringLiteral("Search: Xsgi"));
    QCOMPARE(labelOf(window, QStringLiteral("countLabel"))->text(),
             QStringLiteral("2 entries"));
}

void MainWindowSearchTest::debounceCoalescesKeystrokes()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());

    QSignalSpy searchSpy(&window, &MainWindow::searchEntriesRequested);
    QTest::keyClicks(searchEditOf(window), QStringLiteral("libGL"));

    // Five keystrokes, one request: the final query, once the pause
    // exceeds the debounce interval.
    QTRY_COMPARE(searchSpy.count(), 1);
    QCOMPARE(searchSpy.first().at(1).toString(), QStringLiteral("libGL"));
    QTest::qWait(400);
    QCOMPARE(searchSpy.count(), 1);

    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);
    QCOMPARE(labelOf(window, QStringLiteral("scopeLabel"))->text(),
             QStringLiteral("Search: libGL"));
}

void MainWindowSearchTest::returnKeySearchesImmediately()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());

    QSignalSpy searchSpy(&window, &MainWindow::searchEntriesRequested);
    searchEditOf(window)->setText(QStringLiteral("libGL"));
    QTest::keyClick(searchEditOf(window), Qt::Key_Return);

    // The request left synchronously: no 250 ms debounce wait.
    QCOMPARE(searchSpy.count(), 1);
    QCOMPARE(searchSpy.first().at(1).toString(), QStringLiteral("libGL"));

    // The stopped debounce must not fire a duplicate afterwards.
    QTest::qWait(400);
    QCOMPARE(searchSpy.count(), 1);

    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);
}

void MainWindowSearchTest::zeroResultsShowTheEmptyTable()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());

    searchEditOf(window)->setText(QStringLiteral("definitely-not-a-file"));

    // Zero hits are a successful search: the table page shows an
    // empty model, never the error page.
    auto *stack = stackOf(window.findChild<EntryBrowserWidget *>());
    QTRY_COMPARE(stack->currentWidget()->objectName(), QStringLiteral("tablePage"));
    QCOMPARE(entryTableOf(window)->model()->rowCount(), 0);
    QCOMPARE(labelOf(window, QStringLiteral("countLabel"))->text(),
             QStringLiteral("0 entries"));
    QCOMPARE(labelOf(window, QStringLiteral("scopeLabel"))->text(),
             QStringLiteral("Search: definitely-not-a-file"));
}

void MainWindowSearchTest::clearRestoresHierarchyScope()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    window.show();
    QVERIFY(QTest::qWaitForWindowExposed(&window));

    // Browse the alpha scope first.
    QTreeView *tree = treeOf(window);
    tree->setCurrentIndex(productIndex(tree->model(), QStringLiteral("alpha")));
    QTRY_COMPARE(labelOf(window, QStringLiteral("countLabel"))->text(),
                 QStringLiteral("2 entries"));

    QSignalSpy searchSpy(&window, &MainWindow::searchEntriesRequested);
    QSignalSpy entriesSpy(&window, &MainWindow::entriesRequested);
    QSignalSpy detailSpy(&window, &MainWindow::detailRequested);

    searchEditOf(window)->setText(QStringLiteral("libGL"));
    QTRY_COMPARE(searchSpy.count(), 1);
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);

    // Clearing the field leaves search mode: no empty search request
    // goes out; the still-selected alpha scope reloads instead.
    searchEditOf(window)->clear();
    QCOMPARE(searchSpy.count(), 1);
    QCOMPARE(entriesSpy.count(), 1);
    QCOMPARE(entriesSpy.first().at(1).toULongLong(), 1);
    QCOMPARE(detailSpy.count(), 1);
    QCOMPARE(detailSpy.first().at(1).toULongLong(), 1);

    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 2);
    QCOMPARE(labelOf(window, QStringLiteral("scopeLabel"))->text(), QStringLiteral("alpha"));
}

void MainWindowSearchTest::differentRowClickExitsSearch()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    window.show();
    QVERIFY(QTest::qWaitForWindowExposed(&window));

    QLineEdit *search = searchEditOf(window);
    search->setText(QStringLiteral("libGL"));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);
    QCOMPARE(labelOf(window, QStringLiteral("scopeLabel"))->text(),
             QStringLiteral("Search: libGL"));

    // Clicking another hierarchy row switches the entry browser back
    // to that scope and clears the search field.
    QSignalSpy entriesSpy(&window, &MainWindow::entriesRequested);
    QTreeView *tree = treeOf(window);
    const QModelIndex beta = productIndex(tree->model(), QStringLiteral("beta"));
    QTest::mouseClick(tree->viewport(), Qt::LeftButton, Qt::NoModifier,
                      tree->visualRect(beta).center());

    QCOMPARE(search->text(), QString());
    QCOMPARE(entriesSpy.count(), 1);
    QCOMPARE(entriesSpy.first().at(1).toULongLong(), 5);
    QTRY_COMPARE(entryTableOf(window)->model()->index(0, 0).data().toString(),
                 QStringLiteral("usr/bin/Xsgi"));
    QCOMPARE(entryTableOf(window)->model()->rowCount(), 1);
    QCOMPARE(labelOf(window, QStringLiteral("scopeLabel"))->text(), QStringLiteral("beta"));
}

void MainWindowSearchTest::sameRowClickExitsSearch()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    window.show();
    QVERIFY(QTest::qWaitForWindowExposed(&window));

    // Select alpha, then search: the tree visually keeps the alpha
    // row selected while the entry browser shows global results.
    QTreeView *tree = treeOf(window);
    const QModelIndex alpha = productIndex(tree->model(), QStringLiteral("alpha"));
    tree->setCurrentIndex(alpha);
    QTRY_COMPARE(labelOf(window, QStringLiteral("countLabel"))->text(),
                 QStringLiteral("2 entries"));

    QLineEdit *search = searchEditOf(window);
    search->setText(QStringLiteral("libGL"));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);

    // Clicking the already-current row fires no currentChanged, yet
    // it must still leave search mode and restore the alpha scope.
    QSignalSpy entriesSpy(&window, &MainWindow::entriesRequested);
    QTest::mouseClick(tree->viewport(), Qt::LeftButton, Qt::NoModifier,
                      tree->visualRect(alpha).center());

    QCOMPARE(search->text(), QString());
    QCOMPARE(entriesSpy.count(), 1);
    QCOMPARE(entriesSpy.first().at(1).toULongLong(), 1);
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 2);
    QCOMPARE(labelOf(window, QStringLiteral("scopeLabel"))->text(), QStringLiteral("alpha"));
}

void MainWindowSearchTest::staleEntriesResponseIsDropped()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());

    searchEditOf(window)->setText(QStringLiteral("libGL"));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);

    // A response carrying an unknown request id belongs to a query
    // the user has already left: it must not touch the pane.
    EntrySummarySnapshot bogus;
    bogus.productId = 1;
    bogus.entryId = 0;
    bogus.path = QStringLiteral("bogus/path");
    bogus.subsystem = QStringLiteral("bogus.sw.unix");
    const EntryListSnapshot snapshot = {bogus};

    const bool invoked = QMetaObject::invokeMethod(
        &window, "onEntriesReady", Qt::DirectConnection, Q_ARG(quint64, quint64(999999)),
        Q_ARG(EntryListSnapshot, snapshot));
    QVERIFY(invoked);

    QCOMPARE(entryTableOf(window)->model()->rowCount(), 1);
    QCOMPARE(entryTableOf(window)->model()->index(0, 0).data().toString(),
             QStringLiteral("usr/lib/libGL.so"));
    QCOMPARE(labelOf(window, QStringLiteral("countLabel"))->text(),
             QStringLiteral("1 entries"));
}

void MainWindowSearchTest::searchResultClickShowsInspector()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    window.show();
    QVERIFY(QTest::qWaitForWindowExposed(&window));

    searchEditOf(window)->setText(QStringLiteral("libGL"));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);

    // Clicking a search result drives the same entry-detail pipeline
    // as a hierarchy scope row.
    QTableView *table = entryTableOf(window);
    QTest::mouseClick(table->viewport(), Qt::LeftButton, Qt::NoModifier,
                      table->visualRect(table->model()->index(0, 0)).center());

    auto *inspector = window.findChild<InspectorWidget *>();
    QTRY_VERIFY(inspector->findChild<QScrollArea *>(QStringLiteral("entryPage")) != nullptr);
    QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("nameLabel"))->text(),
             QStringLiteral("usr/lib/libGL.so"));
}

void MainWindowSearchTest::queryChangeInvalidatesInspector()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    window.show();
    QVERIFY(QTest::qWaitForWindowExposed(&window));

    searchEditOf(window)->setText(QStringLiteral("libGL"));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);

    QTableView *table = entryTableOf(window);
    QTest::mouseClick(table->viewport(), Qt::LeftButton, Qt::NoModifier,
                      table->visualRect(table->model()->index(0, 0)).center());
    auto *inspector = window.findChild<InspectorWidget *>();
    QTRY_VERIFY(inspector->findChild<QScrollArea *>(QStringLiteral("entryPage")) != nullptr);

    // A new query invalidates the old entry inspector immediately.
    searchEditOf(window)->setText(QStringLiteral("Xsgi"));
    QCOMPARE(stackOf(inspector)->currentWidget()->objectName(), QStringLiteral("emptyPage"));

    QTRY_COMPARE(table->model()->rowCount(), 2);
}

void MainWindowSearchTest::failedOpenKeepsSearchState()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    MainWindow window;
    loadDistribution(window, dir.path());
    window.show();
    QVERIFY(QTest::qWaitForWindowExposed(&window));

    // Search in distribution A and pick a result for the inspector.
    QLineEdit *search = searchEditOf(window);
    search->setText(QStringLiteral("libGL"));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);
    QTableView *table = entryTableOf(window);
    QTest::mouseClick(table->viewport(), Qt::LeftButton, Qt::NoModifier,
                      table->visualRect(table->model()->index(0, 0)).center());
    auto *inspector = window.findChild<InspectorWidget *>();
    QTRY_VERIFY(inspector->findChild<QScrollArea *>(QStringLiteral("entryPage")) != nullptr);

    // Opening a broken distribution B pops the error box; accept it
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

    // Everything still belongs to A: search text, search results,
    // inspector, tree — and the search box stays usable.
    QCOMPARE(search->text(), QStringLiteral("libGL"));
    QVERIFY(search->isEnabled());
    QCOMPARE(table->model()->rowCount(), 1);
    QCOMPARE(table->model()->index(0, 0).data().toString(),
             QStringLiteral("usr/lib/libGL.so"));
    QCOMPARE(labelOf(window, QStringLiteral("scopeLabel"))->text(),
             QStringLiteral("Search: libGL"));
    QVERIFY(inspector->findChild<QScrollArea *>(QStringLiteral("entryPage")) != nullptr);
    QCOMPARE(treeOf(window)->model()->rowCount(), 2);

    search->setText(QStringLiteral("Xsgi"));
    QTRY_COMPARE(table->model()->rowCount(), 2);
}

void MainWindowSearchTest::successfulReopenClearsSearchState()
{
    QTemporaryDir dirA;
    QVERIFY(writeTwoProductDist(dirA));
    QTemporaryDir dirB;
    QVERIFY(writeGammaDist(dirB));

    MainWindow window;
    loadDistribution(window, dirA.path());

    QLineEdit *search = searchEditOf(window);
    search->setText(QStringLiteral("libGL"));
    QTRY_COMPARE(entryTableOf(window)->model()->rowCount(), 1);

    // The accepted candidate replaces the browsing state wholesale:
    // the search field clears without re-triggering a scope restore,
    // both panes empty, and the new hierarchy arrives.
    window.openDistributionRequested(dirB.path());
    QTRY_COMPARE(treeOf(window)->model()->rowCount(), 1);
    QTRY_VERIFY(treeOf(window)->isEnabled());
    QTRY_VERIFY(search->isEnabled());

    QCOMPARE(search->text(), QString());
    QCOMPARE(stackOf(window.findChild<EntryBrowserWidget *>())->currentWidget()->objectName(),
             QStringLiteral("emptyPage"));
    QCOMPARE(stackOf(window.findChild<InspectorWidget *>())->currentWidget()->objectName(),
             QStringLiteral("emptyPage"));

    // The committed backend serves the new distribution.
    search->setText(QStringLiteral("gamma"));
    QTRY_COMPARE(entryTableOf(window)->model()->index(0, 0).data().toString(),
                 QStringLiteral("usr/bin/gamma"));
    QCOMPARE(entryTableOf(window)->model()->rowCount(), 1);
}

QTEST_MAIN(MainWindowSearchTest)

#include "MainWindowSearchTest.moc"
