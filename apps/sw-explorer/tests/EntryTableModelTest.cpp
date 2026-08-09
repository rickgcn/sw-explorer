#include "models/EntryTableModel.h"

#include <QSignalSpy>
#include <QTest>

// Exercises EntryTableModel with hand-built snapshots: display text,
// raw-value roles, entry keys and the snapshot validation that keeps
// a malformed snapshot from producing a partial render.
class EntryTableModelTest : public QObject
{
    Q_OBJECT

private slots:
    void emptyModel();
    void displayAndHeaderData();
    void sizeAndStoredDisplay();
    void machDisplayKeepsUnresolvedVisible();
    void entryKeyComesFromRowData();
    void entryKeysBeyond32BitsStayDistinct();
    void duplicatePathsStaySeparateRows();
    void resetReplacesRows();
    void duplicateEntryKeysRejectTheSnapshot();
    void statusColumnHeader();
    void inactiveOverlayReportsNoStatus();
    void fourSelectionStates();
    void selectionStatusToolTips();
    void overlayNeverResetsRowsOrOrder();
    void replacingOverlayChangesOnlyStatuses();
    void clearedOverlayReportsNoStatus();
    void entriesReloadPicksUpCurrentOverlay();
    void duplicatePathsKeepIndividualStatuses();
};

namespace {

EntrySummarySnapshot makeEntry(quint64 productId,
                               quint64 entryId,
                               const QString &path,
                               const QString &subsystem)
{
    EntrySummarySnapshot entry;
    entry.productId = productId;
    entry.entryId = entryId;
    entry.path = path;
    entry.subsystem = subsystem;
    entry.fileType = EntryFileType::Regular;
    entry.fileTypeRaw = QStringLiteral("f");
    entry.sizeKnown = true;
    entry.size = 100;
    entry.storedSizeKnown = true;
    entry.storedSize = 100;
    return entry;
}

} // namespace

void EntryTableModelTest::emptyModel()
{
    EntryTableModel model;
    QCOMPARE(model.rowCount(), 0);
    QCOMPARE(model.columnCount(), EntryTableModel::ColumnCount);
    QVERIFY(!model.entryKey(model.index(0, 0)).productId);
    QVERIFY(!model.data(model.index(0, 0)).isValid());
}

void EntryTableModelTest::displayAndHeaderData()
{
    EntryTableModel model;
    EntrySummarySnapshot entry =
        makeEntry(1, 0, QStringLiteral("usr/bin/Xsgi"), QStringLiteral("eoe.sw.unix"));
    QVERIFY(model.setEntries({entry}));

    QCOMPARE(model.headerData(EntryTableModel::PathColumn, Qt::Horizontal).toString(),
             QStringLiteral("Path"));
    QCOMPARE(model.headerData(EntryTableModel::TypeColumn, Qt::Horizontal).toString(),
             QStringLiteral("Type"));
    QCOMPARE(model.headerData(EntryTableModel::SizeColumn, Qt::Horizontal).toString(),
             QStringLiteral("Size"));
    QCOMPARE(model.headerData(EntryTableModel::StoredColumn, Qt::Horizontal).toString(),
             QStringLiteral("Stored"));
    QCOMPARE(model.headerData(EntryTableModel::MachColumn, Qt::Horizontal).toString(),
             QStringLiteral("MACH"));
    QCOMPARE(model.headerData(EntryTableModel::SubsystemColumn, Qt::Horizontal).toString(),
             QStringLiteral("Subsystem"));
    // Vertical headers and non-display roles carry nothing.
    QVERIFY(!model.headerData(0, Qt::Vertical).isValid());
    QVERIFY(!model.headerData(0, Qt::Horizontal, Qt::ToolTipRole).isValid());

    const QModelIndex row = model.index(0, 0);
    QCOMPARE(model.data(row.siblingAtColumn(EntryTableModel::PathColumn)).toString(),
             QStringLiteral("usr/bin/Xsgi"));
    QCOMPARE(model.data(row.siblingAtColumn(EntryTableModel::TypeColumn)).toString(),
             QStringLiteral("File"));
    QCOMPARE(model.data(row.siblingAtColumn(EntryTableModel::SubsystemColumn)).toString(),
             QStringLiteral("eoe.sw.unix"));
    // An unknown type keeps its raw letter visible.
    entry.fileType = EntryFileType::Other;
    entry.fileTypeRaw = QStringLiteral("z");
    QVERIFY(model.setEntries({entry}));
    QCOMPARE(model.data(model.index(0, EntryTableModel::TypeColumn)).toString(),
             QStringLiteral("z"));
}

void EntryTableModelTest::sizeAndStoredDisplay()
{
    EntryTableModel model;
    EntrySummarySnapshot entry = makeEntry(1, 0, QStringLiteral("a"), QStringLiteral("p.sw.s"));
    entry.size = 1363148; // 1.3 MiB
    entry.storedSize = 831488; // exactly 812 KiB
    QVERIFY(model.setEntries({entry}));

    const QModelIndex row = model.index(0, 0);
    QCOMPARE(model.data(row.siblingAtColumn(EntryTableModel::SizeColumn)).toString(),
             QStringLiteral("1.3 MiB"));
    QCOMPARE(model.data(row.siblingAtColumn(EntryTableModel::StoredColumn)).toString(),
             QStringLiteral("812 KiB"));

    // The raw bytes survive in the custom roles and the tooltips.
    QCOMPARE(model.data(row.siblingAtColumn(EntryTableModel::SizeColumn),
                        EntryTableModel::SizeBytesRole)
                 .toULongLong(),
             1363148);
    QCOMPARE(model.data(row.siblingAtColumn(EntryTableModel::StoredColumn),
                        EntryTableModel::StoredBytesRole)
                 .toULongLong(),
             831488);
    QCOMPARE(model.data(row.siblingAtColumn(EntryTableModel::SizeColumn), Qt::ToolTipRole)
                 .toString(),
             QStringLiteral("1363148 bytes"));

    // Sizes are right-aligned.
    QCOMPARE(model.data(row.siblingAtColumn(EntryTableModel::SizeColumn),
                        Qt::TextAlignmentRole)
                 .toInt(),
             static_cast<int>(Qt::AlignRight | Qt::AlignVCenter));
    QCOMPARE(model.data(row.siblingAtColumn(EntryTableModel::PathColumn),
                        Qt::TextAlignmentRole)
                 .isValid(),
             false);

    // Unknown sizes render as a dash, never as a fake zero.
    entry.sizeKnown = false;
    entry.storedSizeKnown = false;
    QVERIFY(model.setEntries({entry}));
    QCOMPARE(model.data(model.index(0, EntryTableModel::SizeColumn)).toString(),
             QStringLiteral("—"));
    QCOMPARE(model.data(model.index(0, EntryTableModel::StoredColumn)).toString(),
             QStringLiteral("—"));
    QVERIFY(!model.data(row.siblingAtColumn(EntryTableModel::SizeColumn),
                        EntryTableModel::SizeBytesRole)
                 .isValid());
}

void EntryTableModelTest::machDisplayKeepsUnresolvedVisible()
{
    EntryTableModel model;
    EntrySummarySnapshot entry = makeEntry(1, 0, QStringLiteral("a"), QStringLiteral("p.sw.s"));
    entry.mach = {QStringLiteral("CPUBOARD=IP22"), QStringLiteral("GFXBOARD=EXPRESS")};
    QVERIFY(model.setEntries({entry}));

    const QModelIndex machIndex = model.index(0, EntryTableModel::MachColumn);
    QCOMPARE(model.data(machIndex).toString(),
             QStringLiteral("CPUBOARD=IP22; GFXBOARD=EXPRESS"));

    // Unresolved MACH must never look like "no MACH": the cell shows
    // a warning marker and the tooltip lists the raw payloads.
    entry.unresolvedMach = {QStringLiteral("=GARBAGE")};
    QVERIFY(model.setEntries({entry}));
    const QString display = model.data(machIndex).toString();
    QVERIFY(display.contains(QStringLiteral("CPUBOARD=IP22")));
    QVERIFY(display.contains(QStringLiteral("unresolved")));
    QVERIFY(!display.contains(QStringLiteral("=GARBAGE")));
    const QString tooltip = model.data(machIndex, Qt::ToolTipRole).toString();
    QVERIFY(tooltip.contains(QStringLiteral("=GARBAGE")));

    // A row with only unresolved MACH still shows the marker.
    entry.mach.clear();
    QVERIFY(model.setEntries({entry}));
    QVERIFY(model.data(machIndex).toString().contains(QStringLiteral("unresolved")));
}

void EntryTableModelTest::entryKeyComesFromRowData()
{
    EntryTableModel model;
    // EntryId 0 is a perfectly valid first entry, never an error.
    QVERIFY(model.setEntries({makeEntry(3, 0, QStringLiteral("a"), QStringLiteral("p.sw.s")),
                              makeEntry(3, 7, QStringLiteral("b"), QStringLiteral("p.sw.s"))}));

    const EntryKey first = model.entryKey(model.index(0, 0));
    QCOMPARE(first.productId, 3);
    QCOMPARE(first.entryId, 0);
    const EntryKey second = model.entryKey(model.index(1, 0));
    QCOMPARE(second.productId, 3);
    QCOMPARE(second.entryId, 7);

    // Out-of-range and foreign indices carry no key.
    QVERIFY(!model.entryKey(QModelIndex()).productId);
    QVERIFY(!model.entryKey(model.index(5, 0)).productId);
}

void EntryTableModelTest::duplicatePathsStaySeparateRows()
{
    EntryTableModel model;
    // MACH variants of one path: three IDB records, three rows.
    EntrySummarySnapshot ip22 =
        makeEntry(1, 0, QStringLiteral("usr/lib/foo.so"), QStringLiteral("eoe.sw.unix"));
    ip22.mach = {QStringLiteral("CPUBOARD=IP22")};
    EntrySummarySnapshot ip26 =
        makeEntry(1, 1, QStringLiteral("usr/lib/foo.so"), QStringLiteral("eoe.sw.unix"));
    ip26.mach = {QStringLiteral("CPUBOARD=IP26")};
    EntrySummarySnapshot ip28 =
        makeEntry(1, 2, QStringLiteral("usr/lib/foo.so"), QStringLiteral("eoe.sw.unix"));
    ip28.mach = {QStringLiteral("CPUBOARD=IP28")};
    QVERIFY(model.setEntries({ip22, ip26, ip28}));

    QCOMPARE(model.rowCount(), 3);
    for (int row = 0; row < 3; ++row) {
        QCOMPARE(model.data(model.index(row, EntryTableModel::PathColumn)).toString(),
                 QStringLiteral("usr/lib/foo.so"));
        QCOMPARE(model.entryKey(model.index(row, 0)).entryId, static_cast<quint64>(row));
    }
}

void EntryTableModelTest::entryKeysBeyond32BitsStayDistinct()
{
    // Both ids are 64-bit. Under a hand-packed (productId << 32) ^
    // entryId these two keys would fold to the same value
    // (2^33 ^ (2^33 + 2^32) == 2^32 == 2^32 ^ 0); as a real pair
    // they are simply different entries and both must be kept.
    EntryTableModel model;
    QVERIFY(model.setEntries({makeEntry(1, 0, QStringLiteral("a"), QStringLiteral("p.sw.s")),
                              makeEntry(2, quint64{0x180000000}, QStringLiteral("b"),
                                        QStringLiteral("q.sw.s"))}));
    QCOMPARE(model.rowCount(), 2);
    QCOMPARE(model.entryKey(model.index(1, 0)).productId, 2);
    QCOMPARE(model.entryKey(model.index(1, 0)).entryId, quint64{0x180000000});
}

void EntryTableModelTest::resetReplacesRows()
{
    EntryTableModel model;
    QVERIFY(model.setEntries({makeEntry(1, 0, QStringLiteral("a"), QStringLiteral("p.sw.s"))}));
    QCOMPARE(model.rowCount(), 1);

    QSignalSpy resetSpy(&model, &QAbstractItemModel::modelReset);
    QVERIFY(model.setEntries({makeEntry(1, 0, QStringLiteral("x"), QStringLiteral("p.sw.s")),
                              makeEntry(1, 1, QStringLiteral("y"), QStringLiteral("p.sw.s"))}));
    QCOMPARE(resetSpy.count(), 1);
    QCOMPARE(model.rowCount(), 2);

    // Resetting to empty is a normal reset, not an error.
    QVERIFY(model.setEntries({}));
    QCOMPARE(model.rowCount(), 0);
}

void EntryTableModelTest::duplicateEntryKeysRejectTheSnapshot()
{
    EntryTableModel model;
    QVERIFY(model.setEntries({makeEntry(1, 0, QStringLiteral("a"), QStringLiteral("p.sw.s"))}));

    // The same (productId, entryId) twice is a bridge bug: the whole
    // snapshot is rejected and the previous rows stay untouched.
    QString error;
    QVERIFY(!model.setEntries({makeEntry(1, 5, QStringLiteral("b"), QStringLiteral("p.sw.s")),
                               makeEntry(1, 5, QStringLiteral("c"), QStringLiteral("p.sw.s"))},
                              &error));
    QVERIFY(error.contains(QStringLiteral("duplicate entry key")));
    QCOMPARE(model.rowCount(), 1);
    QCOMPARE(model.data(model.index(0, EntryTableModel::PathColumn)).toString(),
             QStringLiteral("a"));
}

void EntryTableModelTest::statusColumnHeader()
{
    EntryTableModel model;
    QCOMPARE(model.headerData(EntryTableModel::StatusColumn, Qt::Horizontal).toString(),
             QStringLiteral("Status"));
    QCOMPARE(model.columnCount(), 7);
}

void EntryTableModelTest::inactiveOverlayReportsNoStatus()
{
    EntryTableModel model;
    QVERIFY(model.setEntries({makeEntry(1, 0, QStringLiteral("a"), QStringLiteral("p.sw.s"))}));

    QVERIFY(!model.hasSelectionOverlay());
    QCOMPARE(model.statusForKey({1, 0}), EntrySelectionStatus::NotSelected);
    // Without an overlay the Status column displays nothing at all.
    QVERIFY(!model.data(model.index(0, EntryTableModel::StatusColumn)).isValid());
    QVERIFY(
        !model.data(model.index(0, EntryTableModel::StatusColumn), Qt::ToolTipRole).isValid());
    QCOMPARE(model.selectedRowCount(), 0);
    QCOMPARE(model.conflictedRowCount(), 0);
}

void EntryTableModelTest::fourSelectionStates()
{
    // The four states are pure set membership: selected = {A, B},
    // conflict candidates = {B, C}.
    EntryTableModel model;
    QVERIFY(model.setEntries({makeEntry(1, 0, QStringLiteral("a"), QStringLiteral("p.sw.s")),
                              makeEntry(1, 1, QStringLiteral("b"), QStringLiteral("p.sw.s")),
                              makeEntry(1, 2, QStringLiteral("c"), QStringLiteral("p.sw.s")),
                              makeEntry(1, 3, QStringLiteral("d"), QStringLiteral("p.sw.s"))}));

    SelectionSnapshot selection;
    selection.selected = {{1, 0}, {1, 1}};
    selection.conflicts = {{QStringLiteral("usr/lib/foo.so"), {{1, 1}, {1, 2}}}};
    model.setSelectionOverlay(selection);

    QVERIFY(model.hasSelectionOverlay());
    QCOMPARE(model.statusForKey({1, 0}), EntrySelectionStatus::Selected);
    QCOMPARE(model.statusForKey({1, 1}), EntrySelectionStatus::SelectedConflict);
    QCOMPARE(model.statusForKey({1, 2}), EntrySelectionStatus::Unresolved);
    QCOMPARE(model.statusForKey({1, 3}), EntrySelectionStatus::NotSelected);

    QCOMPARE(model.data(model.index(0, EntryTableModel::StatusColumn)).toString(),
             QStringLiteral("Selected"));
    QCOMPARE(model.data(model.index(1, EntryTableModel::StatusColumn)).toString(),
             QStringLiteral("Selected / conflict"));
    QCOMPARE(model.data(model.index(2, EntryTableModel::StatusColumn)).toString(),
             QStringLiteral("Unresolved"));
    QCOMPARE(model.data(model.index(3, EntryTableModel::StatusColumn)).toString(),
             QStringLiteral("Not selected"));

    QCOMPARE(model
                 .data(model.index(1, EntryTableModel::StatusColumn),
                       EntryTableModel::SelectionStatusRole)
                 .toInt(),
             static_cast<int>(EntrySelectionStatus::SelectedConflict));

    // Footer counters: rows in the selected set, rows in any
    // conflict's candidate set.
    QCOMPARE(model.selectedRowCount(), 2);
    QCOMPARE(model.conflictedRowCount(), 2);
}

void EntryTableModelTest::selectionStatusToolTips()
{
    EntryTableModel model;
    QVERIFY(model.setEntries({makeEntry(1, 0, QStringLiteral("a"), QStringLiteral("p.sw.s")),
                              makeEntry(1, 1, QStringLiteral("b"), QStringLiteral("p.sw.s")),
                              makeEntry(1, 2, QStringLiteral("c"), QStringLiteral("p.sw.s"))}));

    SelectionSnapshot selection;
    selection.selected = {{1, 0}, {1, 1}};
    selection.conflicts = {{QStringLiteral("usr/lib/foo.so"), {{1, 1}, {1, 2}}}};
    model.setSelectionOverlay(selection);

    const auto tip = [&model](int row) {
        return model.data(model.index(row, EntryTableModel::StatusColumn), Qt::ToolTipRole)
            .toString();
    };
    QVERIFY(tip(0).contains(QStringLiteral("is selected for the current hardware profile")));
    QVERIFY(!tip(0).contains(QStringLiteral("conflict")));
    QVERIFY(tip(1).contains(QStringLiteral("usr/lib/foo.so")));
    QVERIFY(tip(1).contains(QStringLiteral("is selected")));
    QVERIFY(tip(2).contains(QStringLiteral("usr/lib/foo.so")));
    QVERIFY(tip(2).contains(QStringLiteral("not selected")));
}

void EntryTableModelTest::overlayNeverResetsRowsOrOrder()
{
    EntryTableModel model;
    QVERIFY(model.setEntries({makeEntry(1, 0, QStringLiteral("a"), QStringLiteral("p.sw.s")),
                              makeEntry(1, 1, QStringLiteral("b"), QStringLiteral("p.sw.s")),
                              makeEntry(1, 2, QStringLiteral("c"), QStringLiteral("p.sw.s"))}));

    const auto shape = [&model] {
        QStringList rows;
        for (int row = 0; row < model.rowCount(); ++row) {
            const EntryKey key = model.entryKey(model.index(row, 0));
            rows.append(QStringLiteral("%1/%2:%3")
                            .arg(key.productId)
                            .arg(key.entryId)
                            .arg(model.data(model.index(row, EntryTableModel::PathColumn))
                                     .toString()));
        }
        return rows;
    };
    const QStringList before = shape();

    // Applying the overlay emits no reset and changes nothing but
    // the Status column.
    QSignalSpy resetSpy(&model, &QAbstractItemModel::modelReset);
    QSignalSpy changedSpy(&model, &QAbstractItemModel::dataChanged);
    SelectionSnapshot selection;
    selection.selected = {{1, 0}, {1, 1}, {1, 2}};
    model.setSelectionOverlay(selection);
    QCOMPARE(resetSpy.count(), 0);
    QCOMPARE(changedSpy.count(), 1);
    QCOMPARE(changedSpy.first().at(0).toModelIndex().column(), EntryTableModel::StatusColumn);
    QCOMPARE(changedSpy.first().at(1).toModelIndex().column(), EntryTableModel::StatusColumn);
    QCOMPARE(shape(), before);

    // Selected rows are never moved up and unselected rows are never
    // hidden.
    selection.selected = {{1, 2}};
    model.setSelectionOverlay(selection);
    QCOMPARE(resetSpy.count(), 0);
    QCOMPARE(shape(), before);
    QCOMPARE(model.data(model.index(2, EntryTableModel::StatusColumn)).toString(),
             QStringLiteral("Selected"));
    QCOMPARE(model.data(model.index(0, EntryTableModel::StatusColumn)).toString(),
             QStringLiteral("Not selected"));

    model.clearSelectionOverlay();
    QCOMPARE(resetSpy.count(), 0);
    QCOMPARE(shape(), before);
}

void EntryTableModelTest::replacingOverlayChangesOnlyStatuses()
{
    EntryTableModel model;
    QVERIFY(model.setEntries({makeEntry(1, 0, QStringLiteral("a"), QStringLiteral("p.sw.s")),
                              makeEntry(1, 1, QStringLiteral("b"), QStringLiteral("p.sw.s"))}));

    SelectionSnapshot first;
    first.selected = {{1, 0}};
    model.setSelectionOverlay(first);
    QCOMPARE(model.statusForKey({1, 0}), EntrySelectionStatus::Selected);
    QCOMPARE(model.statusForKey({1, 1}), EntrySelectionStatus::NotSelected);

    SelectionSnapshot second;
    second.selected = {{1, 1}};
    model.setSelectionOverlay(second);
    QCOMPARE(model.statusForKey({1, 0}), EntrySelectionStatus::NotSelected);
    QCOMPARE(model.statusForKey({1, 1}), EntrySelectionStatus::Selected);
    QCOMPARE(model.rowCount(), 2);
    QCOMPARE(model.selectedRowCount(), 1);
}

void EntryTableModelTest::clearedOverlayReportsNoStatus()
{
    EntryTableModel model;
    QVERIFY(model.setEntries({makeEntry(1, 0, QStringLiteral("a"), QStringLiteral("p.sw.s"))}));

    SelectionSnapshot selection;
    selection.selected = {{1, 0}};
    model.setSelectionOverlay(selection);
    QVERIFY(model.hasSelectionOverlay());

    model.clearSelectionOverlay();
    QVERIFY(!model.hasSelectionOverlay());
    QVERIFY(!model.data(model.index(0, EntryTableModel::StatusColumn)).isValid());
    QCOMPARE(model.selectedRowCount(), 0);
    QCOMPARE(model.conflictedRowCount(), 0);
}

void EntryTableModelTest::entriesReloadPicksUpCurrentOverlay()
{
    EntryTableModel model;
    QVERIFY(model.setEntries({makeEntry(1, 0, QStringLiteral("a"), QStringLiteral("p.sw.s"))}));

    SelectionSnapshot selection;
    selection.selected = {{1, 0}, {1, 7}};
    selection.conflicts = {{QStringLiteral("c"), {{1, 3}}}};
    model.setSelectionOverlay(selection);

    // A new entry list (another scope, a search result) picks the
    // current overlay up by entry key, with no extra work.
    QVERIFY(model.setEntries({makeEntry(1, 7, QStringLiteral("x"), QStringLiteral("p.sw.s")),
                              makeEntry(1, 3, QStringLiteral("y"), QStringLiteral("p.sw.s")),
                              makeEntry(1, 0, QStringLiteral("z"), QStringLiteral("p.sw.s"))}));
    QVERIFY(model.hasSelectionOverlay());
    QCOMPARE(model.statusForKey({1, 7}), EntrySelectionStatus::Selected);
    QCOMPARE(model.statusForKey({1, 3}), EntrySelectionStatus::Unresolved);
    QCOMPARE(model.statusForKey({1, 0}), EntrySelectionStatus::Selected);
    QCOMPARE(model.data(model.index(0, EntryTableModel::StatusColumn)).toString(),
             QStringLiteral("Selected"));
    QCOMPARE(model.data(model.index(1, EntryTableModel::StatusColumn)).toString(),
             QStringLiteral("Unresolved"));
    QCOMPARE(model.selectedRowCount(), 2);
    QCOMPARE(model.conflictedRowCount(), 1);
}

void EntryTableModelTest::duplicatePathsKeepIndividualStatuses()
{
    EntryTableModel model;
    // Two IDB records share one path: each carries its own status.
    QVERIFY(model.setEntries({makeEntry(1, 0, QStringLiteral("usr/lib/foo.so"),
                                        QStringLiteral("p.sw.s")),
                              makeEntry(1, 1, QStringLiteral("usr/lib/foo.so"),
                                        QStringLiteral("p.sw.s"))}));

    SelectionSnapshot selection;
    selection.selected = {{1, 0}};
    model.setSelectionOverlay(selection);

    QCOMPARE(model.rowCount(), 2);
    QCOMPARE(model.data(model.index(0, EntryTableModel::StatusColumn)).toString(),
             QStringLiteral("Selected"));
    QCOMPARE(model.data(model.index(1, EntryTableModel::StatusColumn)).toString(),
             QStringLiteral("Not selected"));
    QCOMPARE(model.data(model.index(0, EntryTableModel::PathColumn)).toString(),
             QStringLiteral("usr/lib/foo.so"));
    QCOMPARE(model.data(model.index(1, EntryTableModel::PathColumn)).toString(),
             QStringLiteral("usr/lib/foo.so"));
}

QTEST_GUILESS_MAIN(EntryTableModelTest)

#include "EntryTableModelTest.moc"


