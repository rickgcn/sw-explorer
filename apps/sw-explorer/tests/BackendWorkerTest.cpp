#include "BackendWorker.h"

#include <QFile>
#include <QSet>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>

// Exercises the candidate/commit protocol of BackendWorker against
// the real Rust backend: the committed backend may only ever change
// through commitCandidate(), and every successful open must hand the
// GUI a fully converted Qt-native snapshot.
class BackendWorkerTest : public QObject
{
    Q_OBJECT

private slots:
    void initTestCase();
    void candidateCarriesHierarchySnapshot();
    void failedOpenEmitsErrorOnly();
    void failedOpenKeepsPendingCandidate();
    void commitAndDiscardWithoutCandidateAreHarmless();
    void commitEmitsCandidateCommitted();
    void detailQueriesRoundTripSnapshots();
    void detailQueryWithoutDistributionFails();
    void detailQueryWithBadIdFails();
    void detailQueryWithWrongKindFails();
    void candidateIsNotQueryable();
    void entriesRoundTripSnapshots();
    void entryDetailRoundTripsSnapshot();
    void entriesWithoutDistributionFail();
    void entriesWithBadScopeFail();
    void entryDetailWithBadKeyFails();
    void candidateEntriesAreNotQueryable();
};

namespace {

// One product `test` with two entries in test.sw.unix and one in
// test.man.man.
bool writeSyntheticDist(QTemporaryDir &dir)
{
    QFile idb(dir.filePath(QStringLiteral("test.idb")));
    if (!idb.open(QIODevice::WriteOnly)) {
        return false;
    }
    idb.write("f 0644 root sys a src/a test.sw.unix sum(1) size(5) cmpsize(0)\n");
    idb.write("f 0644 root sys b src/b test.sw.unix sum(1) size(5) cmpsize(0)\n");
    idb.write("f 0644 root sys c src/c test.man.man sum(1) size(5) cmpsize(0)\n");
    return true;
}

} // namespace

void BackendWorkerTest::initTestCase()
{
    qRegisterMetaType<HierarchyKind>("HierarchyKind");
    qRegisterMetaType<HierarchySnapshot>("HierarchySnapshot");
    qRegisterMetaType<ProductDetailSnapshot>("ProductDetailSnapshot");
    qRegisterMetaType<ImageDetailSnapshot>("ImageDetailSnapshot");
    qRegisterMetaType<SubsystemDetailSnapshot>("SubsystemDetailSnapshot");
    qRegisterMetaType<EntryListSnapshot>("EntryListSnapshot");
    qRegisterMetaType<EntryDetailSnapshot>("EntryDetailSnapshot");
}

void BackendWorkerTest::candidateCarriesHierarchySnapshot()
{
    QTemporaryDir dir;
    QVERIFY(writeSyntheticDist(dir));

    BackendWorker worker;
    QSignalSpy readySpy(&worker, &BackendWorker::candidateReady);
    QSignalSpy failSpy(&worker, &BackendWorker::distributionOpenFailed);

    worker.openDistribution(dir.path());

    QCOMPARE(failSpy.count(), 0);
    QCOMPARE(readySpy.count(), 1);

    const QList<QVariant> &args = readySpy.first();
    QCOMPARE(args.at(0).toULongLong(), 1); // productCount

    const HierarchySnapshot snapshot = qvariant_cast<HierarchySnapshot>(args.at(2));
    QCOMPARE(snapshot.size(), 5);

    QSet<quint64> ids;
    for (const HierarchyNodeSnapshot &node : snapshot) {
        QVERIFY(node.id != 0);
        QVERIFY(!ids.contains(node.id));
        ids.insert(node.id);
    }

    QCOMPARE(snapshot.at(0).kind, HierarchyKind::Product);
    QCOMPARE(snapshot.at(0).name, QStringLiteral("test"));
    QCOMPARE(snapshot.at(0).parentId, quint64(0));
    QCOMPARE(snapshot.at(0).entryCount, 3);
    QCOMPARE(snapshot.at(0).idbPresent, true);
    QCOMPARE(snapshot.at(0).descriptorPresent, false);

    QCOMPARE(snapshot.at(1).kind, HierarchyKind::Image);
    QCOMPARE(snapshot.at(1).name, QStringLiteral("test.sw"));
    QCOMPARE(snapshot.at(1).parentId, snapshot.at(0).id);
    QCOMPARE(snapshot.at(1).entryCount, 2);

    QCOMPARE(snapshot.at(2).kind, HierarchyKind::Subsystem);
    QCOMPARE(snapshot.at(2).name, QStringLiteral("unix"));
    QCOMPARE(snapshot.at(2).parentId, snapshot.at(1).id);
    QCOMPARE(snapshot.at(2).entryCount, 2);

    QCOMPARE(snapshot.at(3).name, QStringLiteral("test.man"));
    QCOMPARE(snapshot.at(3).parentId, snapshot.at(0).id);
    QCOMPARE(snapshot.at(4).name, QStringLiteral("man"));
    QCOMPARE(snapshot.at(4).parentId, snapshot.at(3).id);
}

void BackendWorkerTest::failedOpenEmitsErrorOnly()
{
    BackendWorker worker;
    QSignalSpy readySpy(&worker, &BackendWorker::candidateReady);
    QSignalSpy failSpy(&worker, &BackendWorker::distributionOpenFailed);

    worker.openDistribution(QStringLiteral("/definitely/missing/distribution"));

    QCOMPARE(readySpy.count(), 0);
    QCOMPARE(failSpy.count(), 1);
    QVERIFY(!failSpy.first().at(0).toString().isEmpty());
}

void BackendWorkerTest::failedOpenKeepsPendingCandidate()
{
    QTemporaryDir dir;
    QVERIFY(writeSyntheticDist(dir));

    BackendWorker worker;
    QSignalSpy readySpy(&worker, &BackendWorker::candidateReady);
    QSignalSpy failSpy(&worker, &BackendWorker::distributionOpenFailed);

    // A pending candidate survives a subsequent failed open, and the
    // commit that belongs to it still lands.
    worker.openDistribution(dir.path());
    QCOMPARE(readySpy.count(), 1);

    worker.openDistribution(QStringLiteral("/definitely/missing/distribution"));
    QCOMPARE(failSpy.count(), 1);
    QCOMPARE(readySpy.count(), 1);

    worker.commitCandidate();

    // A fresh open afterwards starts from the committed state.
    worker.openDistribution(dir.path());
    QCOMPARE(readySpy.count(), 2);
}

void BackendWorkerTest::commitAndDiscardWithoutCandidateAreHarmless()
{
    BackendWorker worker;
    worker.commitCandidate();
    worker.discardCandidate();

    QTemporaryDir dir;
    QVERIFY(writeSyntheticDist(dir));
    QSignalSpy readySpy(&worker, &BackendWorker::candidateReady);

    worker.openDistribution(dir.path());
    QCOMPARE(readySpy.count(), 1);

    // Discarding the pending candidate makes a later commit a no-op.
    worker.discardCandidate();
    worker.commitCandidate();

    worker.openDistribution(dir.path());
    QCOMPARE(readySpy.count(), 2);
}

void BackendWorkerTest::commitEmitsCandidateCommitted()
{
    QTemporaryDir dir;
    QVERIFY(writeSyntheticDist(dir));

    BackendWorker worker;
    QSignalSpy committedSpy(&worker, &BackendWorker::candidateCommitted);

    // No candidate pending: no commit, no signal.
    worker.commitCandidate();
    QCOMPARE(committedSpy.count(), 0);

    worker.openDistribution(dir.path());
    worker.commitCandidate();
    QCOMPARE(committedSpy.count(), 1);
}

void BackendWorkerTest::detailQueriesRoundTripSnapshots()
{
    QTemporaryDir dir;
    QVERIFY(writeSyntheticDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy productSpy(&worker, &BackendWorker::productDetailReady);
    QSignalSpy imageSpy(&worker, &BackendWorker::imageDetailReady);
    QSignalSpy subsystemSpy(&worker, &BackendWorker::subsystemDetailReady);
    QSignalSpy failSpy(&worker, &BackendWorker::detailFailed);

    // Object ids from the synthetic tree: 1 = product, 2 = test.sw,
    // 3 = unix, 4 = test.man, 5 = man.
    worker.detailRequested(10, 1, HierarchyKind::Product);
    QCOMPARE(failSpy.count(), 0);
    QCOMPARE(productSpy.count(), 1);
    QCOMPARE(productSpy.first().at(0).toULongLong(), 10);
    const auto product = qvariant_cast<ProductDetailSnapshot>(productSpy.first().at(1));
    QCOMPARE(product.id, 1);
    QCOMPARE(product.name, QStringLiteral("test"));
    QCOMPARE(product.descriptorPresent, false);
    QCOMPARE(product.idbPresent, true);
    QCOMPARE(product.imageCount, 2);
    QCOMPARE(product.subsystemCount, 2);
    QCOMPARE(product.entryCount, 3);
    // No descriptor: MACH is unknown, not "unrestricted".
    QCOMPARE(product.mach.known, false);

    worker.detailRequested(11, 2, HierarchyKind::Image);
    QCOMPARE(imageSpy.count(), 1);
    QCOMPARE(imageSpy.first().at(0).toULongLong(), 11);
    const auto image = qvariant_cast<ImageDetailSnapshot>(imageSpy.first().at(1));
    QCOMPARE(image.name, QStringLiteral("test.sw"));
    QCOMPARE(image.productName, QStringLiteral("test"));
    QCOMPARE(image.versionKnown, false);
    QCOMPARE(image.orderKnown, false);
    QCOMPARE(image.subsystemCount, 1);
    QCOMPARE(image.entryCount, 2);

    worker.detailRequested(12, 3, HierarchyKind::Subsystem);
    QCOMPARE(subsystemSpy.count(), 1);
    QCOMPARE(subsystemSpy.first().at(0).toULongLong(), 12);
    const auto subsystem = qvariant_cast<SubsystemDetailSnapshot>(subsystemSpy.first().at(1));
    QCOMPARE(subsystem.identity, QStringLiteral("test.sw.unix"));
    QCOMPARE(subsystem.shortName, QStringLiteral("unix"));
    QCOMPARE(subsystem.productName, QStringLiteral("test"));
    QCOMPARE(subsystem.imageName, QStringLiteral("test.sw"));
    QCOMPARE(subsystem.entryCount, 2);
    // IDB-only subsystem: every descriptor-derived field is unknown.
    QCOMPARE(subsystem.descriptorPresent, false);
    QCOMPARE(subsystem.idbPresent, true);
    QCOMPARE(subsystem.mappingKnown, false);
    QCOMPARE(subsystem.mach.known, false);
    QCOMPARE(subsystem.flags.known, false);
    QCOMPARE(subsystem.rules.known, false);
    QCOMPARE(subsystem.autominirootKnown, false);
}

void BackendWorkerTest::detailQueryWithoutDistributionFails()
{
    BackendWorker worker;
    QSignalSpy failSpy(&worker, &BackendWorker::detailFailed);

    worker.detailRequested(7, 1, HierarchyKind::Product);
    QCOMPARE(failSpy.count(), 1);
    QCOMPARE(failSpy.first().at(0).toULongLong(), 7);
    QCOMPARE(failSpy.first().at(1).toString(), QStringLiteral("no distribution loaded"));
}

void BackendWorkerTest::detailQueryWithBadIdFails()
{
    QTemporaryDir dir;
    QVERIFY(writeSyntheticDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy failSpy(&worker, &BackendWorker::detailFailed);

    worker.detailRequested(8, 0, HierarchyKind::Product);
    QCOMPARE(failSpy.count(), 1);
    QVERIFY(failSpy.first().at(1).toString().contains(QStringLiteral("object id 0")));

    worker.detailRequested(9, 999, HierarchyKind::Subsystem);
    QCOMPARE(failSpy.count(), 2);
    QVERIFY(failSpy.at(1).at(1).toString().contains(QStringLiteral("does not exist")));
}

void BackendWorkerTest::detailQueryWithWrongKindFails()
{
    QTemporaryDir dir;
    QVERIFY(writeSyntheticDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy failSpy(&worker, &BackendWorker::detailFailed);

    // Object 2 is an image; asking for it as a product must fail
    // with a clear message instead of returning wrong data.
    worker.detailRequested(13, 2, HierarchyKind::Product);
    QCOMPARE(failSpy.count(), 1);
    QCOMPARE(failSpy.first().at(1).toString(),
             QStringLiteral("object 2 is an image, not a product"));
}

void BackendWorkerTest::candidateIsNotQueryable()
{
    // While a candidate awaits validation, detail queries must still
    // be answered by the previously committed backend — or fail when
    // there is none.
    QTemporaryDir dir;
    QVERIFY(writeSyntheticDist(dir));

    BackendWorker worker;
    QSignalSpy failSpy(&worker, &BackendWorker::detailFailed);

    worker.openDistribution(dir.path());
    // The candidate is pending; the committed backend is empty.
    worker.detailRequested(14, 1, HierarchyKind::Product);
    QCOMPARE(failSpy.count(), 1);
    QCOMPARE(failSpy.first().at(1).toString(), QStringLiteral("no distribution loaded"));

    // After the commit the same query succeeds.
    QSignalSpy productSpy(&worker, &BackendWorker::productDetailReady);
    worker.commitCandidate();
    worker.detailRequested(15, 1, HierarchyKind::Product);
    QCOMPARE(productSpy.count(), 1);
    QCOMPARE(qvariant_cast<ProductDetailSnapshot>(productSpy.first().at(1)).name,
             QStringLiteral("test"));
}

void BackendWorkerTest::entriesRoundTripSnapshots()
{
    QTemporaryDir dir;
    QVERIFY(writeSyntheticDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy entriesSpy(&worker, &BackendWorker::entriesReady);
    QSignalSpy failSpy(&worker, &BackendWorker::entriesFailed);

    // Object ids from the synthetic tree: 1 = product, 2 = test.sw,
    // 3 = unix, 4 = test.man, 5 = man.

    // Product scope: every entry, in exact IDB order.
    worker.entriesRequested(30, 1);
    QCOMPARE(failSpy.count(), 0);
    QCOMPARE(entriesSpy.count(), 1);
    QCOMPARE(entriesSpy.first().at(0).toULongLong(), 30);
    const auto productEntries = qvariant_cast<EntryListSnapshot>(entriesSpy.first().at(1));
    QCOMPARE(productEntries.size(), 3);
    QCOMPARE(productEntries.at(0).path, QStringLiteral("a"));
    QCOMPARE(productEntries.at(1).path, QStringLiteral("b"));
    QCOMPARE(productEntries.at(2).path, QStringLiteral("c"));
    for (qsizetype i = 0; i < productEntries.size(); ++i) {
        QCOMPARE(productEntries.at(i).productId, 1);
        QCOMPARE(productEntries.at(i).entryId, static_cast<quint64>(i));
    }
    // size(5) cmpsize(0): stored uncompressed, so the stored size is
    // the plain size — never "stored size 0".
    QCOMPARE(productEntries.at(0).sizeKnown, true);
    QCOMPARE(productEntries.at(0).size, 5);
    QCOMPARE(productEntries.at(0).storedSizeKnown, true);
    QCOMPARE(productEntries.at(0).storedSize, 5);
    QCOMPARE(productEntries.at(0).fileType, EntryFileType::Regular);

    // Image scope filters the product entries, keeping their order.
    worker.entriesRequested(31, 2);
    QCOMPARE(entriesSpy.count(), 2);
    const auto imageEntries = qvariant_cast<EntryListSnapshot>(entriesSpy.at(1).at(1));
    QCOMPARE(imageEntries.size(), 2);
    QCOMPARE(imageEntries.at(0).path, QStringLiteral("a"));
    QCOMPARE(imageEntries.at(1).path, QStringLiteral("b"));
    QCOMPARE(imageEntries.at(0).subsystem, QStringLiteral("test.sw.unix"));

    // Subsystem scope.
    worker.entriesRequested(32, 3);
    QCOMPARE(entriesSpy.count(), 3);
    const auto unixEntries = qvariant_cast<EntryListSnapshot>(entriesSpy.at(2).at(1));
    QCOMPARE(unixEntries.size(), 2);

    worker.entriesRequested(33, 5);
    QCOMPARE(entriesSpy.count(), 4);
    const auto manEntries = qvariant_cast<EntryListSnapshot>(entriesSpy.at(3).at(1));
    QCOMPARE(manEntries.size(), 1);
    QCOMPARE(manEntries.at(0).path, QStringLiteral("c"));
    QCOMPARE(manEntries.at(0).entryId, 2);
    QCOMPARE(manEntries.at(0).subsystem, QStringLiteral("test.man.man"));

    QCOMPARE(failSpy.count(), 0);
}

void BackendWorkerTest::entryDetailRoundTripsSnapshot()
{
    QTemporaryDir dir;
    QVERIFY(writeSyntheticDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy detailSpy(&worker, &BackendWorker::entryDetailReady);
    QSignalSpy failSpy(&worker, &BackendWorker::detailFailed);

    // EntryId 0 is a perfectly valid first entry.
    worker.entryDetailRequested(40, 1, 0);
    QCOMPARE(failSpy.count(), 0);
    QCOMPARE(detailSpy.count(), 1);
    QCOMPARE(detailSpy.first().at(0).toULongLong(), 40);
    const auto detail = qvariant_cast<EntryDetailSnapshot>(detailSpy.first().at(1));
    QCOMPARE(detail.productId, 1);
    QCOMPARE(detail.entryId, 0);
    QCOMPARE(detail.fileType, EntryFileType::Regular);
    QCOMPARE(detail.fileTypeRaw, QStringLiteral("f"));
    QCOMPARE(detail.mode, 0644);
    QCOMPARE(detail.owner, QStringLiteral("root"));
    QCOMPARE(detail.group, QStringLiteral("sys"));
    QCOMPARE(detail.path, QStringLiteral("a"));
    QCOMPARE(detail.rawPath, QStringLiteral("a"));
    QCOMPARE(detail.sourcePath, QStringLiteral("src/a"));
    QCOMPARE(detail.subsystem, QStringLiteral("test.sw.unix"));
    QCOMPARE(detail.sizeKnown, true);
    QCOMPARE(detail.size, 5);
    QCOMPARE(detail.compressedSizeKnown, true);
    QCOMPARE(detail.compressedSize, 0);
    QCOMPARE(detail.storedSizeKnown, true);
    QCOMPARE(detail.storedSize, 5);
    QCOMPARE(detail.checksumKnown, true);
    QCOMPARE(detail.checksum, 1);
    // No optional attributes: unknown, not empty fakes.
    QCOMPARE(detail.configKnown, false);
    QCOMPARE(detail.symlinkTargetKnown, false);
    QCOMPARE(detail.deviceKnown, false);
    // The payload locator is layout metadata computed from the IDB
    // alone; no archive was read for it. This entry is the first
    // payload record of test.sw, stored uncompressed.
    QCOMPARE(detail.payloadPresent, true);
    QCOMPARE(detail.payloadImage, QStringLiteral("test.sw"));
    QCOMPARE(detail.payloadEncodedSizeKnown, true);
    QCOMPARE(detail.payloadEncodedSize, 5);
    QCOMPARE(detail.expectedRecordOffsetKnown, true);
    QCOMPARE(detail.expectedRecordOffset, 13);
    // The raw IDB record round-trips as the lossless fallback.
    QVERIFY(detail.originIdbPath.contains(QStringLiteral("test.idb")));
    QCOMPARE(detail.originLine, 1);
    QVERIFY(detail.rawIdbLine.startsWith(QStringLiteral("f 0644 root sys a")));
}

void BackendWorkerTest::entriesWithoutDistributionFail()
{
    BackendWorker worker;
    QSignalSpy entriesFailSpy(&worker, &BackendWorker::entriesFailed);
    QSignalSpy detailFailSpy(&worker, &BackendWorker::detailFailed);

    worker.entriesRequested(50, 1);
    QCOMPARE(entriesFailSpy.count(), 1);
    QCOMPARE(entriesFailSpy.first().at(0).toULongLong(), 50);
    QCOMPARE(entriesFailSpy.first().at(1).toString(),
             QStringLiteral("no distribution loaded"));

    worker.entryDetailRequested(51, 1, 0);
    QCOMPARE(detailFailSpy.count(), 1);
    QCOMPARE(detailFailSpy.first().at(0).toULongLong(), 51);
    QCOMPARE(detailFailSpy.first().at(1).toString(),
             QStringLiteral("no distribution loaded"));
}

void BackendWorkerTest::entriesWithBadScopeFail()
{
    QTemporaryDir dir;
    QVERIFY(writeSyntheticDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy failSpy(&worker, &BackendWorker::entriesFailed);

    worker.entriesRequested(60, 0);
    QCOMPARE(failSpy.count(), 1);
    QVERIFY(failSpy.first().at(1).toString().contains(QStringLiteral("object id 0")));

    worker.entriesRequested(61, 999);
    QCOMPARE(failSpy.count(), 2);
    QVERIFY(failSpy.at(1).at(1).toString().contains(QStringLiteral("does not exist")));
}

void BackendWorkerTest::entryDetailWithBadKeyFails()
{
    QTemporaryDir dir;
    QVERIFY(writeSyntheticDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy failSpy(&worker, &BackendWorker::detailFailed);

    // The product id must identify a product, not any other object.
    worker.entryDetailRequested(70, 2, 0);
    QCOMPARE(failSpy.count(), 1);
    QCOMPARE(failSpy.first().at(1).toString(),
             QStringLiteral("object 2 is an image, not a product"));

    // An out-of-range entry id names the product it was sought in.
    worker.entryDetailRequested(71, 1, 99);
    QCOMPARE(failSpy.count(), 2);
    QCOMPARE(failSpy.at(1).at(1).toString(),
             QStringLiteral("entry 99 does not exist in product test"));
}

void BackendWorkerTest::candidateEntriesAreNotQueryable()
{
    // Like every other query, entry queries only ever talk to the
    // committed backend; a pending candidate is not queryable.
    QTemporaryDir dir;
    QVERIFY(writeSyntheticDist(dir));

    BackendWorker worker;
    QSignalSpy entriesFailSpy(&worker, &BackendWorker::entriesFailed);
    QSignalSpy detailFailSpy(&worker, &BackendWorker::detailFailed);

    worker.openDistribution(dir.path());
    // The candidate is pending; the committed backend is empty.
    worker.entriesRequested(80, 1);
    QCOMPARE(entriesFailSpy.count(), 1);
    QCOMPARE(entriesFailSpy.first().at(1).toString(),
             QStringLiteral("no distribution loaded"));
    worker.entryDetailRequested(81, 1, 0);
    QCOMPARE(detailFailSpy.count(), 1);

    // After the commit the same queries succeed.
    QSignalSpy entriesSpy(&worker, &BackendWorker::entriesReady);
    QSignalSpy detailSpy(&worker, &BackendWorker::entryDetailReady);
    worker.commitCandidate();
    worker.entriesRequested(82, 1);
    QCOMPARE(entriesSpy.count(), 1);
    QCOMPARE(qvariant_cast<EntryListSnapshot>(entriesSpy.first().at(1)).size(), 3);
    worker.entryDetailRequested(83, 1, 0);
    QCOMPARE(detailSpy.count(), 1);
    QCOMPARE(qvariant_cast<EntryDetailSnapshot>(detailSpy.first().at(1)).path,
             QStringLiteral("a"));
}

QTEST_GUILESS_MAIN(BackendWorkerTest)

#include "BackendWorkerTest.moc"
