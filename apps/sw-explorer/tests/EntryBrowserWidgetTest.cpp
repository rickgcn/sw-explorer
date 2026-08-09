#include "EntryBrowserWidget.h"

#include <QLabel>
#include <QSignalSpy>
#include <QStackedWidget>
#include <QTableView>
#include <QTest>

#include "models/EntryTableModel.h"

// Drives EntryBrowserWidget with hand-built snapshots: the page
// stack, the entry count, and the selection protocol — an explicit
// row pick emits entrySelected, a model reset never does.
class EntryBrowserWidgetTest : public QObject
{
    Q_OBJECT

private slots:
    void emptyPageIsInitial();
    void loadingPageKeepsScopeName();
    void entriesShowTableWithCount();
    void errorPageShowsMessage();
    void rowSelectionEmitsEntryKey();
    void reloadDoesNotEmitSelection();
    void invalidSnapshotShowsError();
    void successfulReloadRecoversFromError();
};

namespace {

EntrySummarySnapshot makeEntry(quint64 productId, quint64 entryId, const QString &path)
{
    EntrySummarySnapshot entry;
    entry.productId = productId;
    entry.entryId = entryId;
    entry.path = path;
    entry.subsystem = QStringLiteral("eoe.sw.unix");
    entry.fileType = EntryFileType::Regular;
    entry.fileTypeRaw = QStringLiteral("f");
    return entry;
}

EntryListSnapshot twoEntries()
{
    return {makeEntry(1, 0, QStringLiteral("usr/bin/Xsgi")),
            makeEntry(1, 1, QStringLiteral("usr/lib/libGL.so"))};
}

QStackedWidget *stackOf(EntryBrowserWidget &widget)
{
    return widget.findChild<QStackedWidget *>();
}

QString currentPageName(EntryBrowserWidget &widget)
{
    return stackOf(widget)->currentWidget()->objectName();
}

} // namespace

void EntryBrowserWidgetTest::emptyPageIsInitial()
{
    EntryBrowserWidget widget;
    QCOMPARE(currentPageName(widget), QStringLiteral("emptyPage"));

    widget.showEmpty();
    QCOMPARE(currentPageName(widget), QStringLiteral("emptyPage"));
}

void EntryBrowserWidgetTest::loadingPageKeepsScopeName()
{
    EntryBrowserWidget widget;
    widget.showLoading(QStringLiteral("eoe.sw.unix"));
    QCOMPARE(currentPageName(widget), QStringLiteral("loadingPage"));

    // The scope name survives into the table header once the data
    // arrives.
    widget.showEntries(twoEntries());
    auto *scopeLabel = widget.findChild<QLabel *>(QStringLiteral("scopeLabel"));
    QVERIFY(scopeLabel != nullptr);
    QCOMPARE(scopeLabel->text(), QStringLiteral("eoe.sw.unix"));
    QCOMPARE(scopeLabel->textFormat(), Qt::PlainText);
}

void EntryBrowserWidgetTest::entriesShowTableWithCount()
{
    EntryBrowserWidget widget;
    widget.showLoading(QStringLiteral("eoe.sw.unix"));
    widget.showEntries(twoEntries());

    QCOMPARE(currentPageName(widget), QStringLiteral("tablePage"));
    auto *countLabel = widget.findChild<QLabel *>(QStringLiteral("countLabel"));
    QVERIFY(countLabel != nullptr);
    QCOMPARE(countLabel->text(), QStringLiteral("2 entries"));

    auto *table = widget.findChild<QTableView *>(QStringLiteral("entryTable"));
    QVERIFY(table != nullptr);
    // The view order is the IDB order; sorting stays off.
    QVERIFY(!table->isSortingEnabled());
    QCOMPARE(table->model()->rowCount(), 2);
}

void EntryBrowserWidgetTest::errorPageShowsMessage()
{
    EntryBrowserWidget widget;
    widget.showError(QStringLiteral("object 12 does not exist"));
    QCOMPARE(currentPageName(widget), QStringLiteral("errorPage"));

    auto *message = widget.findChild<QLabel *>(QStringLiteral("errorMessage"));
    QVERIFY(message != nullptr);
    QCOMPARE(message->text(), QStringLiteral("object 12 does not exist"));
    QCOMPARE(message->textFormat(), Qt::PlainText);
}

void EntryBrowserWidgetTest::rowSelectionEmitsEntryKey()
{
    EntryBrowserWidget widget;
    widget.showEntries(twoEntries());

    qRegisterMetaType<quint64>("quint64");
    QSignalSpy spy(&widget, &EntryBrowserWidget::entrySelected);
    auto *table = widget.findChild<QTableView *>(QStringLiteral("entryTable"));

    table->setCurrentIndex(table->model()->index(1, 0));
    QCOMPARE(spy.count(), 1);
    QCOMPARE(spy.first().at(0).toULongLong(), 1);
    QCOMPARE(spy.first().at(1).toULongLong(), 1);

    table->setCurrentIndex(table->model()->index(0, 0));
    QCOMPARE(spy.count(), 2);
    QCOMPARE(spy.at(1).at(0).toULongLong(), 1);
    // EntryId 0 is a perfectly valid id, emitted like any other.
    QCOMPARE(spy.at(1).at(1).toULongLong(), 0);
}

void EntryBrowserWidgetTest::reloadDoesNotEmitSelection()
{
    EntryBrowserWidget widget;
    widget.showEntries(twoEntries());

    qRegisterMetaType<quint64>("quint64");
    QSignalSpy spy(&widget, &EntryBrowserWidget::entrySelected);
    auto *table = widget.findChild<QTableView *>(QStringLiteral("entryTable"));
    table->setCurrentIndex(table->model()->index(0, 0));
    QCOMPARE(spy.count(), 1);

    // A reload drops the selection through a model reset; that must
    // not emit anything, so the inspector keeps showing the
    // hierarchy detail the user last picked.
    widget.showEntries(twoEntries());
    QCOMPARE(spy.count(), 1);
    QVERIFY(!table->selectionModel()->currentIndex().isValid());

    // Loading and empty transitions stay silent too.
    widget.showLoading(QStringLiteral("eoe.sw.gfx"));
    widget.showEmpty();
    QCOMPARE(spy.count(), 1);
}

void EntryBrowserWidgetTest::invalidSnapshotShowsError()
{
    EntryBrowserWidget widget;
    widget.showEntries(twoEntries());
    QCOMPARE(currentPageName(widget), QStringLiteral("tablePage"));

    // Duplicate entry keys: the backend returned garbage. Nothing is
    // replaced and the error page explains the problem.
    EntryListSnapshot invalid = {makeEntry(1, 0, QStringLiteral("a")),
                                 makeEntry(1, 0, QStringLiteral("b"))};
    widget.showEntries(invalid);
    QCOMPARE(currentPageName(widget), QStringLiteral("errorPage"));
    auto *message = widget.findChild<QLabel *>(QStringLiteral("errorMessage"));
    QVERIFY(message->text().contains(QStringLiteral("invalid entry list")));

    auto *table = widget.findChild<QTableView *>(QStringLiteral("entryTable"));
    QCOMPARE(table->model()->rowCount(), 2); // untouched
}

void EntryBrowserWidgetTest::successfulReloadRecoversFromError()
{
    EntryBrowserWidget widget;
    widget.showError(QStringLiteral("object 12 does not exist"));
    QCOMPARE(currentPageName(widget), QStringLiteral("errorPage"));

    widget.showLoading(QStringLiteral("eoe.sw.unix"));
    widget.showEntries(twoEntries());
    QCOMPARE(currentPageName(widget), QStringLiteral("tablePage"));
    QCOMPARE(widget.findChild<QLabel *>(QStringLiteral("countLabel"))->text(),
             QStringLiteral("2 entries"));
}

QTEST_MAIN(EntryBrowserWidgetTest)

#include "EntryBrowserWidgetTest.moc"
