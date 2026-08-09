#include "models/HardwareProfileModel.h"

#include <QTest>

// Exercises HardwareProfileModel with hand-built profiles: editing,
// adding and removing rows, multi-valued attributes and the profile
// roundtrip.
class HardwareProfileModelTest : public QObject
{
    Q_OBJECT

private slots:
    void emptyModel();
    void setAndReadBackProfile();
    void addAndRemoveRows();
    void editAttributeAndValue();
    void multiValuedAttributeStaysSeparateRows();
    void profileRoundTrip();
    void exactDuplicatePairsAreKeptVerbatim();
    void editingTrimsOuterWhitespace();
    void unknownAttributesAndValuesAreKept();
};

void HardwareProfileModelTest::emptyModel()
{
    HardwareProfileModel model;
    QCOMPARE(model.rowCount(), 0);
    QCOMPARE(model.columnCount(), HardwareProfileModel::ColumnCount);
    QVERIFY(model.profile().isEmpty());
    QVERIFY(!model.data(model.index(0, 0)).isValid());
}

void HardwareProfileModelTest::setAndReadBackProfile()
{
    HardwareProfileModel model;
    const HardwareProfileSnapshot profile = {{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")},
                                             {QStringLiteral("GFXBOARD"),
                                              QStringLiteral("EXPRESS")}};
    model.setProfile(profile);

    QCOMPARE(model.rowCount(), 2);
    QCOMPARE(model.headerData(HardwareProfileModel::AttributeColumn, Qt::Horizontal).toString(),
             QStringLiteral("Attribute"));
    QCOMPARE(model.headerData(HardwareProfileModel::ValueColumn, Qt::Horizontal).toString(),
             QStringLiteral("Value"));
    QCOMPARE(model.data(model.index(0, HardwareProfileModel::AttributeColumn)).toString(),
             QStringLiteral("CPUBOARD"));
    QCOMPARE(model.data(model.index(0, HardwareProfileModel::ValueColumn)).toString(),
             QStringLiteral("IP22"));
    const HardwareProfileSnapshot readBack = model.profile();
    QCOMPARE(readBack.size(), 2);
    QCOMPARE(readBack.at(0).attribute, QStringLiteral("CPUBOARD"));
    QCOMPARE(readBack.at(0).value, QStringLiteral("IP22"));
    QCOMPARE(readBack.at(1).attribute, QStringLiteral("GFXBOARD"));
    QCOMPARE(readBack.at(1).value, QStringLiteral("EXPRESS"));
}

void HardwareProfileModelTest::addAndRemoveRows()
{
    HardwareProfileModel model;
    model.setProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});

    const QModelIndex added = model.addRow();
    QCOMPARE(model.rowCount(), 2);
    QCOMPARE(added.row(), 1);
    QCOMPARE(added.column(), HardwareProfileModel::AttributeColumn);
    QVERIFY(model.flags(added) & Qt::ItemIsEditable);

    // Removing several rows at once, given in any order.
    model.addRow();
    model.addRow();
    QCOMPARE(model.rowCount(), 4);
    model.removeRows({3, 1});
    QCOMPARE(model.rowCount(), 2);
    QCOMPARE(model.profile().size(), 2);
    QCOMPARE(model.profile().at(0).attribute, QStringLiteral("CPUBOARD"));
    QCOMPARE(model.profile().at(1).attribute, QString());

    // Out-of-range rows are ignored.
    model.removeRows({42, -1});
    QCOMPARE(model.rowCount(), 2);

    model.clear();
    QCOMPARE(model.rowCount(), 0);
    QVERIFY(model.profile().isEmpty());
}

void HardwareProfileModelTest::editAttributeAndValue()
{
    HardwareProfileModel model;
    model.setProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});

    QVERIFY(model.setData(model.index(0, HardwareProfileModel::AttributeColumn),
                          QStringLiteral("GFXBOARD")));
    QVERIFY(model.setData(model.index(0, HardwareProfileModel::ValueColumn),
                          QStringLiteral("EXPRESS")));
    QCOMPARE(model.profile().at(0).attribute, QStringLiteral("GFXBOARD"));
    QCOMPARE(model.profile().at(0).value, QStringLiteral("EXPRESS"));

    // Display role edits and out-of-range indices are rejected.
    QVERIFY(!model.setData(model.index(0, 0), QStringLiteral("X"), Qt::DisplayRole));
    QVERIFY(!model.setData(model.index(9, 0), QStringLiteral("X")));
}

void HardwareProfileModelTest::multiValuedAttributeStaysSeparateRows()
{
    HardwareProfileModel model;
    // One attribute, two values: both rows survive.
    model.setProfile({{QStringLiteral("CPUARCH"), QStringLiteral("MIPS2")},
                      {QStringLiteral("CPUARCH"), QStringLiteral("R4000")}});

    QCOMPARE(model.rowCount(), 2);
    QCOMPARE(model.profile().at(0).attribute, QStringLiteral("CPUARCH"));
    QCOMPARE(model.profile().at(0).value, QStringLiteral("MIPS2"));
    QCOMPARE(model.profile().at(1).attribute, QStringLiteral("CPUARCH"));
    QCOMPARE(model.profile().at(1).value, QStringLiteral("R4000"));
}

void HardwareProfileModelTest::profileRoundTrip()
{
    HardwareProfileModel model;
    HardwareProfileModel copy;
    const HardwareProfileSnapshot profile = {{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")},
                                             {QStringLiteral("CPUARCH"), QStringLiteral("MIPS2")},
                                             {QStringLiteral("CPUARCH"), QStringLiteral("R4000")},
                                             {QStringLiteral("MODE"), QStringLiteral("32bit")}};
    model.setProfile(profile);
    copy.setProfile(model.profile());
    QCOMPARE(copy.profile().size(), 4);
    for (int row = 0; row < profile.size(); ++row) {
        QCOMPARE(copy.profile().at(row).attribute, profile.at(row).attribute);
        QCOMPARE(copy.profile().at(row).value, profile.at(row).value);
    }
}

void HardwareProfileModelTest::exactDuplicatePairsAreKeptVerbatim()
{
    HardwareProfileModel model;
    // The model stores rows faithfully; exact duplicates are
    // deduplicated harmlessly by the core profile builder, not here.
    model.setProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")},
                      {QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});
    QCOMPARE(model.rowCount(), 2);
    QCOMPARE(model.profile().at(0).attribute, model.profile().at(1).attribute);
    QCOMPARE(model.profile().at(0).value, model.profile().at(1).value);
}

void HardwareProfileModelTest::editingTrimsOuterWhitespace()
{
    HardwareProfileModel model;
    model.setProfile({{QString(), QString()}});

    QVERIFY(model.setData(model.index(0, HardwareProfileModel::AttributeColumn),
                          QStringLiteral("  CPUBOARD ")));
    QVERIFY(model.setData(model.index(0, HardwareProfileModel::ValueColumn),
                          QStringLiteral("\tIP22\n")));
    QCOMPARE(model.profile().at(0).attribute, QStringLiteral("CPUBOARD"));
    QCOMPARE(model.profile().at(0).value, QStringLiteral("IP22"));
}

void HardwareProfileModelTest::unknownAttributesAndValuesAreKept()
{
    HardwareProfileModel model;
    // No candidate list is consulted: unknown names pass through.
    model.setProfile({{QStringLiteral("FROBNICATE"), QStringLiteral("YES")}});
    QCOMPARE(model.profile().at(0).attribute, QStringLiteral("FROBNICATE"));
    QCOMPARE(model.profile().at(0).value, QStringLiteral("YES"));
}

QTEST_GUILESS_MAIN(HardwareProfileModelTest)

#include "HardwareProfileModelTest.moc"
