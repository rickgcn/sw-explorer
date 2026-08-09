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

QTEST_GUILESS_MAIN(EntryTableModelTest)

#include "EntryTableModelTest.moc"
