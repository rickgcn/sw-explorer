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
    qRegisterMetaType<HierarchySnapshot>("HierarchySnapshot");
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

QTEST_GUILESS_MAIN(BackendWorkerTest)

#include "BackendWorkerTest.moc"
