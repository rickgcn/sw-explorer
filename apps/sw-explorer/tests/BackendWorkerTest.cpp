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
    void searchRoundTripsSnapshots();
    void searchExplicitWildcardIsNotWrapped();
    void searchZeroResultsIsNotAnError();
    void searchEmptyQueryFails();
    void searchWithoutDistributionFails();
    void candidateSearchIsNotQueryable();
    void searchResultKeysResolveEntryDetail();
    void hardwareCandidatesRoundTrip();
    void hardwareCandidatesWithoutDistributionFail();
    void candidateHardwareQueriesAreNotQueryable();
    void pendingCandidateKeepsCommittedHardwareAnswers();
    void selectionRoundTripsSnapshot();
    void selectionMultiValuedProfileReachesRust();
    void selectionUnknownAttributePassesThrough();
    void selectionEmptyValuePassesThrough();
    void selectionRejectsEmptyAttributeName();
    void selectionWithoutDistributionFails();
    void selectionKeysResolveEntryDetail();
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

// Two products: `alpha` with usr/bin/Xsgi in alpha.sw.unix and
// usr/lib/libGL.so in alpha.sw.gfx, `beta` with a second
// usr/bin/Xsgi record in beta.sw.unix. Object ids: 1 = alpha,
// 2 = alpha.sw, 3 = unix, 4 = gfx, 5 = beta, 6 = beta.sw, 7 = unix.
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

// One product `gamma` with a single usr/bin/gamma record; the
// contents never overlap with writeTwoProductDist.
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

// One product `hw` whose records exercise mach-specific selection,
// mach-less fallback, a multi-value CPUARCH conflict, an unknown
// attribute and an unparseable payload. Object ids: 1 = product `hw`,
// 2 = image `hw.sw`, 3 = `unix`.
bool writeHardwareDist(QTemporaryDir &dir)
{
    QFile idb(dir.filePath(QStringLiteral("hw.idb")));
    if (!idb.open(QIODevice::WriteOnly)) {
        return false;
    }
    idb.write(
        "f 0755 root sys usr/bin/plain src/plain hw.sw.unix sum(1) size(10) cmpsize(0)\n"
        "f 0755 root sys usr/bin/board src/board hw.sw.unix sum(1) size(10) cmpsize(0) mach(CPUBOARD=IP22)\n"
        "f 0644 root sys usr/lib/dup src/dup1 hw.sw.unix sum(1) size(10) cmpsize(0) mach(CPUBOARD=IP22)\n"
        "f 0644 root sys usr/lib/dup src/dup2 hw.sw.unix sum(1) size(10) cmpsize(0)\n"
        "f 0644 root sys usr/lib/conf src/conf4 hw.sw.unix sum(1) size(10) cmpsize(0) mach(CPUARCH=R4000)\n"
        "f 0644 root sys usr/lib/conf src/conf5 hw.sw.unix sum(1) size(10) cmpsize(0) mach(CPUARCH=R5000)\n"
        "f 0755 root sys usr/bin/frob src/frob hw.sw.unix sum(1) size(10) cmpsize(0) mach(FROBNICATE=YES)\n"
        "f 0644 root sys usr/lib/broken src/broken hw.sw.unix sum(1) size(10) cmpsize(0) mach(=GARBAGE)\n");
    return true;
}

// One product `gfx` with a mach-less record and one record restricted
// to headless boards through `mach(GFXBOARD=)` — the empty right-hand
// side real media carry. Object ids: 1 = product `gfx`, 2 = `gfx.sw`,
// 3 = `unix`.
bool writeEmptyValueHardwareDist(QTemporaryDir &dir)
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

// "productId/entryId" per key, for readable QCOMPARE diffs.
QStringList keyTexts(const QList<EntryKey> &keys)
{
    QStringList out;
    out.reserve(keys.size());
    for (const EntryKey &key : keys) {
        out.append(QStringLiteral("%1/%2").arg(key.productId).arg(key.entryId));
    }
    return out;
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
    qRegisterMetaType<HardwareProfileSnapshot>("HardwareProfileSnapshot");
    qRegisterMetaType<HardwareCandidatesSnapshot>("HardwareCandidatesSnapshot");
    qRegisterMetaType<SelectionSnapshot>("SelectionSnapshot");
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

void BackendWorkerTest::searchRoundTripsSnapshots()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy entriesSpy(&worker, &BackendWorker::entriesReady);
    QSignalSpy failSpy(&worker, &BackendWorker::entriesFailed);

    // A search spans every product: Xsgi lives in alpha and beta.
    worker.searchEntriesRequested(90, QStringLiteral("Xsgi"));
    QCOMPARE(failSpy.count(), 0);
    QCOMPARE(entriesSpy.count(), 1);
    QCOMPARE(entriesSpy.first().at(0).toULongLong(), 90);
    const auto hits = qvariant_cast<EntryListSnapshot>(entriesSpy.first().at(1));
    QCOMPARE(hits.size(), 2);
    // Distribution product order, each product in IDB order.
    QCOMPARE(hits.at(0).productId, 1);
    QCOMPARE(hits.at(0).entryId, 0);
    QCOMPARE(hits.at(0).path, QStringLiteral("usr/bin/Xsgi"));
    QCOMPARE(hits.at(0).subsystem, QStringLiteral("alpha.sw.unix"));
    QCOMPARE(hits.at(0).storedSize, 60);
    QCOMPARE(hits.at(1).productId, 5);
    QCOMPARE(hits.at(1).entryId, 0);
    QCOMPARE(hits.at(1).path, QStringLiteral("usr/bin/Xsgi"));
    QCOMPARE(hits.at(1).subsystem, QStringLiteral("beta.sw.unix"));

    // Plain text is a substring match: libGL hits one record.
    worker.searchEntriesRequested(91, QStringLiteral("libGL"));
    QCOMPARE(entriesSpy.count(), 2);
    const auto gl = qvariant_cast<EntryListSnapshot>(entriesSpy.at(1).at(1));
    QCOMPARE(gl.size(), 1);
    QCOMPARE(gl.at(0).productId, 1);
    QCOMPARE(gl.at(0).entryId, 1);
    QCOMPARE(gl.at(0).path, QStringLiteral("usr/lib/libGL.so"));
    QCOMPARE(failSpy.count(), 0);
}

void BackendWorkerTest::searchExplicitWildcardIsNotWrapped()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy entriesSpy(&worker, &BackendWorker::entriesReady);

    // A pattern with its own wildcard reaches the matcher unchanged:
    // "libGL*" requires the path to start with libGL, so nothing
    // matches; a wrapped pattern would hit usr/lib/libGL.so.
    worker.searchEntriesRequested(95, QStringLiteral("libGL*"));
    QCOMPARE(entriesSpy.count(), 1);
    QCOMPARE(qvariant_cast<EntryListSnapshot>(entriesSpy.first().at(1)).size(), 0);

    // "*.so" matches only paths ending in .so.
    worker.searchEntriesRequested(96, QStringLiteral("*.so"));
    QCOMPARE(entriesSpy.count(), 2);
    const auto hits = qvariant_cast<EntryListSnapshot>(entriesSpy.at(1).at(1));
    QCOMPARE(hits.size(), 1);
    QCOMPARE(hits.at(0).path, QStringLiteral("usr/lib/libGL.so"));
}

void BackendWorkerTest::searchZeroResultsIsNotAnError()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy entriesSpy(&worker, &BackendWorker::entriesReady);
    QSignalSpy failSpy(&worker, &BackendWorker::entriesFailed);

    worker.searchEntriesRequested(97, QStringLiteral("definitely-not-a-file"));
    QCOMPARE(failSpy.count(), 0);
    QCOMPARE(entriesSpy.count(), 1);
    QCOMPARE(qvariant_cast<EntryListSnapshot>(entriesSpy.first().at(1)).size(), 0);
}

void BackendWorkerTest::searchEmptyQueryFails()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy entriesSpy(&worker, &BackendWorker::entriesReady);
    QSignalSpy failSpy(&worker, &BackendWorker::entriesFailed);

    worker.searchEntriesRequested(98, QString());
    QCOMPARE(entriesSpy.count(), 0);
    QCOMPARE(failSpy.count(), 1);
    QCOMPARE(failSpy.first().at(0).toULongLong(), 98);
    QCOMPARE(failSpy.first().at(1).toString(), QStringLiteral("search query is empty"));
}

void BackendWorkerTest::searchWithoutDistributionFails()
{
    BackendWorker worker;
    QSignalSpy failSpy(&worker, &BackendWorker::entriesFailed);

    worker.searchEntriesRequested(99, QStringLiteral("Xsgi"));
    QCOMPARE(failSpy.count(), 1);
    QCOMPARE(failSpy.first().at(0).toULongLong(), 99);
    QCOMPARE(failSpy.first().at(1).toString(), QStringLiteral("no distribution loaded"));
}

void BackendWorkerTest::candidateSearchIsNotQueryable()
{
    // Like every other query, a search only ever talks to the
    // committed backend; a pending candidate is not queryable.
    QTemporaryDir dirA;
    QVERIFY(writeTwoProductDist(dirA));
    QTemporaryDir dirB;
    QVERIFY(writeGammaDist(dirB));

    BackendWorker worker;
    worker.openDistribution(dirA.path());
    worker.commitCandidate();

    // A pending candidate stays invisible: gamma only exists in the
    // candidate, the committed backend still answers with alpha/beta.
    worker.openDistribution(dirB.path());

    QSignalSpy entriesSpy(&worker, &BackendWorker::entriesReady);
    QSignalSpy failSpy(&worker, &BackendWorker::entriesFailed);

    worker.searchEntriesRequested(100, QStringLiteral("gamma"));
    QCOMPARE(entriesSpy.count(), 1);
    QCOMPARE(qvariant_cast<EntryListSnapshot>(entriesSpy.first().at(1)).size(), 0);

    worker.searchEntriesRequested(101, QStringLiteral("Xsgi"));
    QCOMPARE(entriesSpy.count(), 2);
    QCOMPARE(qvariant_cast<EntryListSnapshot>(entriesSpy.at(1).at(1)).size(), 2);

    // After the commit the same queries see the new distribution.
    worker.commitCandidate();
    worker.searchEntriesRequested(102, QStringLiteral("gamma"));
    QCOMPARE(entriesSpy.count(), 3);
    const auto gamma = qvariant_cast<EntryListSnapshot>(entriesSpy.at(2).at(1));
    QCOMPARE(gamma.size(), 1);
    QCOMPARE(gamma.at(0).path, QStringLiteral("usr/bin/gamma"));

    worker.searchEntriesRequested(103, QStringLiteral("Xsgi"));
    QCOMPARE(entriesSpy.count(), 4);
    QCOMPARE(qvariant_cast<EntryListSnapshot>(entriesSpy.at(3).at(1)).size(), 0);
    QCOMPARE(failSpy.count(), 0);
}

void BackendWorkerTest::searchResultKeysResolveEntryDetail()
{
    QTemporaryDir dir;
    QVERIFY(writeTwoProductDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy entriesSpy(&worker, &BackendWorker::entriesReady);
    QSignalSpy detailSpy(&worker, &BackendWorker::entryDetailReady);
    QSignalSpy detailFailSpy(&worker, &BackendWorker::detailFailed);

    worker.searchEntriesRequested(110, QStringLiteral("Xsgi"));
    QCOMPARE(entriesSpy.count(), 1);
    const auto hits = qvariant_cast<EntryListSnapshot>(entriesSpy.first().at(1));
    QCOMPARE(hits.size(), 2);

    // The (productId, entryId) key of every hit feeds the inspector
    // query directly.
    for (qsizetype i = 0; i < hits.size(); ++i) {
        worker.entryDetailRequested(111 + i, hits.at(i).productId, hits.at(i).entryId);
    }
    QCOMPARE(detailFailSpy.count(), 0);
    QCOMPARE(detailSpy.count(), 2);
    const auto first = qvariant_cast<EntryDetailSnapshot>(detailSpy.first().at(1));
    QCOMPARE(first.path, QStringLiteral("usr/bin/Xsgi"));
    QCOMPARE(first.subsystem, QStringLiteral("alpha.sw.unix"));
    QCOMPARE(first.size, 100);
    QCOMPARE(first.storedSize, 60);
    const auto second = qvariant_cast<EntryDetailSnapshot>(detailSpy.at(1).at(1));
    QCOMPARE(second.path, QStringLiteral("usr/bin/Xsgi"));
    QCOMPARE(second.subsystem, QStringLiteral("beta.sw.unix"));
}

void BackendWorkerTest::hardwareCandidatesRoundTrip()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy readySpy(&worker, &BackendWorker::hardwareCandidatesReady);
    QSignalSpy failSpy(&worker, &BackendWorker::hardwareCandidatesFailed);

    worker.hardwareCandidatesRequested(120);
    QCOMPARE(failSpy.count(), 0);
    QCOMPARE(readySpy.count(), 1);
    QCOMPARE(readySpy.first().at(0).toULongLong(), 120);

    const auto candidates =
        qvariant_cast<HardwareCandidatesSnapshot>(readySpy.first().at(1));
    QCOMPARE(candidates.size(), 3);
    // Entry MACH candidates, in IDB first-appearance order; the
    // unknown attribute is preserved verbatim; the unparseable
    // payload contributes nothing.
    QCOMPARE(candidates.at(0).attribute, QStringLiteral("CPUBOARD"));
    QCOMPARE(candidates.at(0).values, QStringList{QStringLiteral("IP22")});
    QCOMPARE(candidates.at(1).attribute, QStringLiteral("CPUARCH"));
    QCOMPARE(candidates.at(1).values,
             (QStringList{QStringLiteral("R4000"), QStringLiteral("R5000")}));
    QCOMPARE(candidates.at(2).attribute, QStringLiteral("FROBNICATE"));
    QCOMPARE(candidates.at(2).values, QStringList{QStringLiteral("YES")});
}

void BackendWorkerTest::hardwareCandidatesWithoutDistributionFail()
{
    BackendWorker worker;
    QSignalSpy readySpy(&worker, &BackendWorker::hardwareCandidatesReady);
    QSignalSpy failSpy(&worker, &BackendWorker::hardwareCandidatesFailed);

    worker.hardwareCandidatesRequested(121);
    QCOMPARE(readySpy.count(), 0);
    QCOMPARE(failSpy.count(), 1);
    QCOMPARE(failSpy.first().at(0).toULongLong(), 121);
    QCOMPARE(failSpy.first().at(1).toString(), QStringLiteral("no distribution loaded"));
}

void BackendWorkerTest::candidateHardwareQueriesAreNotQueryable()
{
    // A pending candidate is never queried: with no committed
    // backend at all, hardware queries fail like any other query.
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());

    QSignalSpy candidatesFailSpy(&worker, &BackendWorker::hardwareCandidatesFailed);
    QSignalSpy selectionFailSpy(&worker, &BackendWorker::selectionFailed);

    worker.hardwareCandidatesRequested(122);
    QCOMPARE(candidatesFailSpy.count(), 1);
    QCOMPARE(candidatesFailSpy.first().at(1).toString(),
             QStringLiteral("no distribution loaded"));

    worker.selectionRequested(123, {{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});
    QCOMPARE(selectionFailSpy.count(), 1);
    QCOMPARE(selectionFailSpy.first().at(1).toString(),
             QStringLiteral("no distribution loaded"));
}

void BackendWorkerTest::pendingCandidateKeepsCommittedHardwareAnswers()
{
    QTemporaryDir dirA;
    QVERIFY(writeHardwareDist(dirA));
    QTemporaryDir dirB;
    QVERIFY(writeGammaDist(dirB));

    BackendWorker worker;
    worker.openDistribution(dirA.path());
    worker.commitCandidate();

    // A pending candidate B stays invisible: both hardware queries
    // keep answering with the committed A.
    worker.openDistribution(dirB.path());

    QSignalSpy candidatesSpy(&worker, &BackendWorker::hardwareCandidatesReady);
    QSignalSpy selectionSpy(&worker, &BackendWorker::selectionReady);

    worker.hardwareCandidatesRequested(124);
    QCOMPARE(candidatesSpy.count(), 1);
    QCOMPARE(qvariant_cast<HardwareCandidatesSnapshot>(candidatesSpy.first().at(1)).size(), 3);

    worker.selectionRequested(125, {{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});
    QCOMPARE(selectionSpy.count(), 1);
    auto selection = qvariant_cast<SelectionSnapshot>(selectionSpy.first().at(1));
    QCOMPARE(selection.selected.size(), 3);
    QVERIFY(selection.selected.at(0).productId == 1);

    // After the commit the same queries see B.
    worker.commitCandidate();
    worker.hardwareCandidatesRequested(126);
    QCOMPARE(candidatesSpy.count(), 2);
    QVERIFY(qvariant_cast<HardwareCandidatesSnapshot>(candidatesSpy.at(1).at(1)).isEmpty());

    worker.selectionRequested(127, {{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});
    QCOMPARE(selectionSpy.count(), 2);
    selection = qvariant_cast<SelectionSnapshot>(selectionSpy.at(1).at(1));
    QCOMPARE(selection.selected.size(), 1);
}

void BackendWorkerTest::selectionRoundTripsSnapshot()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy readySpy(&worker, &BackendWorker::selectionReady);
    QSignalSpy failSpy(&worker, &BackendWorker::selectionFailed);

    worker.selectionRequested(130, {{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});
    QCOMPARE(failSpy.count(), 0);
    QCOMPARE(readySpy.count(), 1);
    QCOMPARE(readySpy.first().at(0).toULongLong(), 130);

    const auto selection = qvariant_cast<SelectionSnapshot>(readySpy.first().at(1));
    // The plain fallback, the IP22 entry and the IP22-specific
    // duplicate; the unparsed record conflicts but is not selected.
    QCOMPARE(keyTexts(selection.selected),
             (QStringList{QStringLiteral("1/0"), QStringLiteral("1/1"), QStringLiteral("1/2")}));
    QCOMPARE(selection.conflicts.size(), 1);
    QCOMPARE(selection.conflicts.at(0).path, QStringLiteral("usr/lib/broken"));
    QCOMPARE(keyTexts(selection.conflicts.at(0).candidates),
             QStringList{QStringLiteral("1/7")});
}

void BackendWorkerTest::selectionMultiValuedProfileReachesRust()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy readySpy(&worker, &BackendWorker::selectionReady);

    // One attribute carried by two pairs: both CPUARCH-specific
    // records match, so both are selected and the path conflicts.
    worker.selectionRequested(131,
                              {{QStringLiteral("CPUARCH"), QStringLiteral("R4000")},
                               {QStringLiteral("CPUARCH"), QStringLiteral("R5000")}});
    QCOMPARE(readySpy.count(), 1);
    const auto selection = qvariant_cast<SelectionSnapshot>(readySpy.first().at(1));
    QVERIFY(selection.selected.contains({1, 4}));
    QVERIFY(selection.selected.contains({1, 5}));
    QCOMPARE(selection.conflicts.size(), 2);
    QCOMPARE(selection.conflicts.at(0).path, QStringLiteral("usr/lib/conf"));
    QCOMPARE(keyTexts(selection.conflicts.at(0).candidates),
             (QStringList{QStringLiteral("1/4"), QStringLiteral("1/5")}));
}

void BackendWorkerTest::selectionUnknownAttributePassesThrough()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy readySpy(&worker, &BackendWorker::selectionReady);
    QSignalSpy failSpy(&worker, &BackendWorker::selectionFailed);

    // An unknown attribute is not whitelisted away: it reaches the
    // core profile and selects the FROBNICATE record.
    worker.selectionRequested(132, {{QStringLiteral("FROBNICATE"), QStringLiteral("YES")}});
    QCOMPARE(failSpy.count(), 0);
    QCOMPARE(readySpy.count(), 1);
    const auto selection = qvariant_cast<SelectionSnapshot>(readySpy.first().at(1));
    QVERIFY(selection.selected.contains({1, 6}));
    QVERIFY(!selection.selected.contains({1, 1}));
}

void BackendWorkerTest::selectionEmptyValuePassesThrough()
{
    QTemporaryDir dir;
    QVERIFY(writeEmptyValueHardwareDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy readySpy(&worker, &BackendWorker::selectionReady);
    QSignalSpy failSpy(&worker, &BackendWorker::selectionFailed);

    // The empty value is a genuine hardware fact: it reaches the core
    // profile untouched and selects the `GFXBOARD=` record.
    worker.selectionRequested(136, {{QStringLiteral("GFXBOARD"), QString()}});
    QCOMPARE(failSpy.count(), 0);
    QCOMPARE(readySpy.count(), 1);
    auto selection = qvariant_cast<SelectionSnapshot>(readySpy.first().at(1));
    QVERIFY(selection.selected.contains({1, 0}));
    QVERIFY(selection.selected.contains({1, 1}));

    // A non-empty value rejects the headless record; the mach-less
    // one is selected either way.
    worker.selectionRequested(137, {{QStringLiteral("GFXBOARD"), QStringLiteral("EXPRESS")}});
    QCOMPARE(readySpy.count(), 2);
    selection = qvariant_cast<SelectionSnapshot>(readySpy.at(1).at(1));
    QVERIFY(selection.selected.contains({1, 0}));
    QVERIFY(!selection.selected.contains({1, 1}));
}

void BackendWorkerTest::selectionRejectsEmptyAttributeName()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy readySpy(&worker, &BackendWorker::selectionReady);
    QSignalSpy failSpy(&worker, &BackendWorker::selectionFailed);

    worker.selectionRequested(133, {{QString(), QStringLiteral("IP22")}});
    QCOMPARE(failSpy.count(), 1);
    QCOMPARE(failSpy.first().at(1).toString(),
             QStringLiteral("hardware attribute name is empty"));
    QCOMPARE(readySpy.count(), 0);
}

void BackendWorkerTest::selectionWithoutDistributionFails()
{
    BackendWorker worker;
    QSignalSpy failSpy(&worker, &BackendWorker::selectionFailed);

    worker.selectionRequested(135, {{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});
    QCOMPARE(failSpy.count(), 1);
    QCOMPARE(failSpy.first().at(0).toULongLong(), 135);
    QCOMPARE(failSpy.first().at(1).toString(), QStringLiteral("no distribution loaded"));
}

void BackendWorkerTest::selectionKeysResolveEntryDetail()
{
    QTemporaryDir dir;
    QVERIFY(writeHardwareDist(dir));

    BackendWorker worker;
    worker.openDistribution(dir.path());
    worker.commitCandidate();

    QSignalSpy readySpy(&worker, &BackendWorker::selectionReady);
    QSignalSpy detailSpy(&worker, &BackendWorker::entryDetailReady);
    QSignalSpy detailFailSpy(&worker, &BackendWorker::detailFailed);

    worker.selectionRequested(140, {{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});
    QCOMPARE(readySpy.count(), 1);
    const auto selection = qvariant_cast<SelectionSnapshot>(readySpy.first().at(1));

    // Every selected and conflicted key feeds the inspector query
    // directly, including entry id 0.
    QList<EntryKey> keys = selection.selected;
    for (const SelectionConflictSnapshot &conflict : selection.conflicts) {
        keys.append(conflict.candidates);
    }
    for (qsizetype i = 0; i < keys.size(); ++i) {
        worker.entryDetailRequested(141 + i, keys.at(i).productId, keys.at(i).entryId);
    }
    QCOMPARE(detailFailSpy.count(), 0);
    QCOMPARE(detailSpy.count(), keys.size());
    QCOMPARE(qvariant_cast<EntryDetailSnapshot>(detailSpy.first().at(1)).path,
             QStringLiteral("usr/bin/plain"));
}

QTEST_GUILESS_MAIN(BackendWorkerTest)

#include "BackendWorkerTest.moc"
