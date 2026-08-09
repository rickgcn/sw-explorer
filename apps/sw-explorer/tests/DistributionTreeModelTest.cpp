#include "models/DistributionTreeModel.h"

#include <QTest>

class DistributionTreeModelTest : public QObject
{
    Q_OBJECT

private slots:
    void emptyModel();
    void validSnapshotBuildsTree();
    void displayAndTooltipRoles();
    void objectIdentity();
    void resetReplacesTree();
    void malformedSnapshotsKeepOldTree();
};

namespace {

HierarchyNodeSnapshot makeNode(quint64 id,
                               quint64 parentId,
                               HierarchyKind kind,
                               const QString &name,
                               quint64 entryCount,
                               const QString &title = QString(),
                               bool descriptorPresent = true,
                               bool idbPresent = true)
{
    HierarchyNodeSnapshot node;
    node.id = id;
    node.parentId = parentId;
    node.kind = kind;
    node.name = name;
    node.title = title;
    node.entryCount = entryCount;
    node.descriptorPresent = descriptorPresent;
    node.idbPresent = idbPresent;
    return node;
}

// eoe(160)
// +-- eoe.sw(150)
// |   +-- unix(100)
// |   +-- base(50)
// +-- eoe.man(10)
//     +-- relnotes(10)
HierarchySnapshot sampleSnapshot()
{
    return {
        makeNode(1,
                 0,
                 HierarchyKind::Product,
                 QStringLiteral("eoe"),
                 160,
                 QStringLiteral("Execution Only Environment")),
        makeNode(2,
                 1,
                 HierarchyKind::Image,
                 QStringLiteral("eoe.sw"),
                 150,
                 QStringLiteral("System Software")),
        makeNode(3,
                 2,
                 HierarchyKind::Subsystem,
                 QStringLiteral("unix"),
                 100,
                 QStringLiteral("UNIX Kernel")),
        makeNode(4, 2, HierarchyKind::Subsystem, QStringLiteral("base"), 50),
        makeNode(5, 1, HierarchyKind::Image, QStringLiteral("eoe.man"), 10),
        makeNode(6, 5, HierarchyKind::Subsystem, QStringLiteral("relnotes"), 10),
    };
}

} // namespace

void DistributionTreeModelTest::emptyModel()
{
    DistributionTreeModel model;
    QCOMPARE(model.columnCount(), 2);
    QCOMPARE(model.rowCount(), 0);
    QVERIFY(!model.index(0, 0).isValid());
    QCOMPARE(model.objectId(QModelIndex()), quint64(0));
    QCOMPARE(model.headerData(0, Qt::Horizontal).toString(), QStringLiteral("Name"));
    QCOMPARE(model.headerData(1, Qt::Horizontal).toString(), QStringLiteral("Entries"));
}

void DistributionTreeModelTest::validSnapshotBuildsTree()
{
    DistributionTreeModel model;
    QString error;
    QVERIFY2(model.setHierarchy(sampleSnapshot(), &error), qPrintable(error));

    QCOMPARE(model.rowCount(), 1);
    const QModelIndex product = model.index(0, 0);
    QVERIFY(product.isValid());
    QCOMPARE(model.rowCount(product), 2);

    const QModelIndex sw = model.index(0, 0, product);
    const QModelIndex man = model.index(1, 0, product);
    QCOMPARE(model.rowCount(sw), 2);
    QCOMPARE(model.rowCount(man), 1);

    const QModelIndex unix = model.index(0, 0, sw);
    QVERIFY(unix.isValid());
    QCOMPARE(model.rowCount(unix), 0);

    // parent() walks back up to the invisible root.
    QCOMPARE(model.parent(unix), sw);
    QCOMPARE(model.parent(sw), product);
    QVERIFY(!model.parent(product).isValid());

    // index() and parent() are exact inverses.
    const QModelIndex base = model.index(1, 0, sw);
    QCOMPARE(base.data().toString(), QStringLiteral("base"));
    QCOMPARE(model.parent(base), sw);

    // Only the name column carries children.
    QCOMPARE(model.columnCount(sw), 2);
    QCOMPARE(model.rowCount(model.index(0, 1, product)), 0);
}

void DistributionTreeModelTest::displayAndTooltipRoles()
{
    DistributionTreeModel model;
    QVERIFY(model.setHierarchy(sampleSnapshot()));

    const QModelIndex product = model.index(0, 0);
    const QModelIndex sw = model.index(0, 0, product);
    const QModelIndex unix = model.index(0, 0, sw);
    const QModelIndex base = model.index(1, 0, sw);

    // DisplayRole: name column and entries column.
    QCOMPARE(model.data(product).toString(), QStringLiteral("eoe"));
    QCOMPARE(model.data(sw).toString(), QStringLiteral("eoe.sw"));
    QCOMPARE(model.data(unix).toString(), QStringLiteral("unix"));
    QCOMPARE(model.data(model.index(0, 1)).toULongLong(), 160);
    QCOMPARE(model.data(model.index(0, 1, product)).toULongLong(), 150);
    QCOMPARE(model.data(model.index(0, 1, sw)).toULongLong(), 100);

    // ToolTipRole: full identity, title, presence flags, entries.
    const QString unixTip = model.data(unix, Qt::ToolTipRole).toString();
    QVERIFY(unixTip.contains(QStringLiteral("eoe.sw.unix")));
    QVERIFY(unixTip.contains(QStringLiteral("UNIX Kernel")));
    QVERIFY(unixTip.contains(QStringLiteral("Descriptor: yes")));
    QVERIFY(unixTip.contains(QStringLiteral("IDB: yes")));
    QVERIFY(unixTip.contains(QStringLiteral("Entries: 100")));

    // An item without a title must not show an empty title line.
    const QString baseTip = model.data(base, Qt::ToolTipRole).toString();
    QVERIFY(baseTip.startsWith(QStringLiteral("eoe.sw.base\n")));
    QVERIFY(!baseTip.contains(QStringLiteral("\n\n")));
}

void DistributionTreeModelTest::objectIdentity()
{
    DistributionTreeModel model;
    QVERIFY(model.setHierarchy(sampleSnapshot()));

    const QModelIndex product = model.index(0, 0);
    const QModelIndex sw = model.index(0, 0, product);
    const QModelIndex unix = model.index(0, 0, sw);

    QCOMPARE(model.objectId(product), quint64(1));
    QCOMPARE(model.kind(product), HierarchyKind::Product);
    QCOMPARE(model.identity(product), QStringLiteral("eoe"));

    QCOMPARE(model.objectId(sw), quint64(2));
    QCOMPARE(model.kind(sw), HierarchyKind::Image);
    QCOMPARE(model.identity(sw), QStringLiteral("eoe.sw"));

    QCOMPARE(model.objectId(unix), quint64(3));
    QCOMPARE(model.kind(unix), HierarchyKind::Subsystem);
    QCOMPARE(model.identity(unix), QStringLiteral("eoe.sw.unix"));

    // The entries column of the same row addresses the same object.
    QCOMPARE(model.objectId(model.index(0, 1, sw)), quint64(3));
}

void DistributionTreeModelTest::resetReplacesTree()
{
    DistributionTreeModel model;
    QVERIFY(model.setHierarchy(sampleSnapshot()));
    QCOMPARE(model.rowCount(), 1);

    const HierarchySnapshot replacement = {
        makeNode(7, 0, HierarchyKind::Product, QStringLiteral("demo"), 0),
    };
    QVERIFY(model.setHierarchy(replacement));
    QCOMPARE(model.rowCount(), 1);
    const QModelIndex product = model.index(0, 0);
    QCOMPARE(product.data().toString(), QStringLiteral("demo"));
    QCOMPARE(model.objectId(product), quint64(7));
    QCOMPARE(model.rowCount(product), 0);

    // An empty snapshot is a valid, empty tree.
    QVERIFY(model.setHierarchy({}));
    QCOMPARE(model.rowCount(), 0);
}

void DistributionTreeModelTest::malformedSnapshotsKeepOldTree()
{
    DistributionTreeModel model;
    QVERIFY(model.setHierarchy(sampleSnapshot()));

    const auto expectRejected = [&model](const HierarchySnapshot &bad) {
        QString error;
        QVERIFY(!model.setHierarchy(bad, &error));
        QVERIFY(!error.isEmpty());
        // The previously loaded tree is untouched.
        QCOMPARE(model.rowCount(), 1);
        QCOMPARE(model.index(0, 0).data().toString(), QStringLiteral("eoe"));
        QCOMPARE(model.rowCount(model.index(0, 0)), 2);
    };

    // Id 0 is reserved for the invisible root.
    expectRejected({makeNode(0, 0, HierarchyKind::Product, QStringLiteral("x"), 0)});

    // Duplicate ids.
    HierarchySnapshot duplicate = sampleSnapshot();
    duplicate.append(makeNode(3, 0, HierarchyKind::Product, QStringLiteral("dup"), 0));
    expectRejected(duplicate);

    // A product must not have a parent.
    expectRejected({makeNode(1, 0, HierarchyKind::Product, QStringLiteral("ok"), 0),
                    makeNode(2, 1, HierarchyKind::Product, QStringLiteral("nested"), 0)});

    // An image must hang below a product.
    expectRejected({makeNode(1, 0, HierarchyKind::Product, QStringLiteral("ok"), 0),
                    makeNode(2, 1, HierarchyKind::Image, QStringLiteral("ok.sw"), 0),
                    makeNode(3, 2, HierarchyKind::Image, QStringLiteral("ok.man"), 0)});

    // A subsystem must hang below an image.
    expectRejected({makeNode(1, 0, HierarchyKind::Product, QStringLiteral("ok"), 0),
                    makeNode(2, 1, HierarchyKind::Subsystem, QStringLiteral("unix"), 0)});

    // The parent must exist.
    expectRejected({makeNode(1, 0, HierarchyKind::Product, QStringLiteral("ok"), 0),
                    makeNode(2, 99, HierarchyKind::Image, QStringLiteral("ok.sw"), 0)});

    // A non-product must not dangle from the root.
    expectRejected({makeNode(1, 0, HierarchyKind::Image, QStringLiteral("orphan.sw"), 0)});
}

QTEST_GUILESS_MAIN(DistributionTreeModelTest)

#include "DistributionTreeModelTest.moc"
