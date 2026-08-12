#include "SessionState.h"

#include <QTest>

// Unit tests for the GUI session state machine: the open transaction
// phases, the committed/pending distribution split, the browse mode,
// the hardware profile/selection lifecycle, the extraction lock and
// the per-family request generations. Pure value state — no widgets,
// no backend.
class SessionStateTest : public QObject
{
    Q_OBJECT

private slots:
    void startupState();
    void requestFamiliesAreIndependent();
    void failedReopenPreservesSession();
    void rejectedCandidatePreservesSession();
    void acceptedReplacementStagesPending();
    void commitPublishesPending();
    void emptyProfileReplacementStaysInactive();
    void profileLifecycle();
    void extractionLifecycle();
};

namespace {

HardwareProfileSnapshot ip22Profile()
{
    return {{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}};
}

HardwareCandidatesSnapshot someCandidates()
{
    return {{QStringLiteral("CPUBOARD"), {QStringLiteral("IP22"), QStringLiteral("IP32")}}};
}

// Drives a session to: committed distribution A, IP22 profile,
// candidates present, a Ready selection and search mode active.
void driveToCommittedSession(SessionState &session)
{
    session.applyProfile(ip22Profile());
    session.beginOpen();
    session.acceptCandidate({2, 4});
    session.candidateCommitted();
    session.setCandidates(someCandidates());
    session.selectionSucceeded(6, 1);
    session.enterSearch();
}

} // namespace

void SessionStateTest::startupState()
{
    const SessionState session;

    QCOMPARE(session.openPhase(), OpenPhase::Idle);
    QVERIFY(!session.hasDistribution());
    QVERIFY(!session.committedDistribution().has_value());
    QVERIFY(!session.displayedDistributionSummary().has_value());
    QCOMPARE(session.browseMode(), BrowseMode::Hierarchy);
    QVERIFY(!session.searchActive());
    QVERIFY(session.profile().isEmpty());
    QVERIFY(session.candidates().isEmpty());
    QCOMPARE(session.selectionPhase(), SelectionPhase::Inactive);
    QVERIFY(session.selectionError().isEmpty());
    QCOMPARE(session.selectedRecordCount(), 0);
    QCOMPARE(session.conflictGroupCount(), 0);
    QVERIFY(!session.extractionRunning());
}

void SessionStateTest::requestFamiliesAreIndependent()
{
    RequestGenerations requests;

    // Issuing bumps only the chosen family.
    const quint64 entriesId = requests.issue(RequestFamily::Entries);
    QCOMPARE(entriesId, 1);
    QCOMPARE(requests.current(RequestFamily::Entries), 1);
    QCOMPARE(requests.current(RequestFamily::Inspector), 0);
    QCOMPARE(requests.current(RequestFamily::Selection), 0);
    QCOMPARE(requests.current(RequestFamily::HardwareCandidates), 0);
    QCOMPARE(requests.current(RequestFamily::Extraction), 0);

    QVERIFY(requests.accepts(RequestFamily::Entries, entriesId));

    // Invalidating makes the outstanding id stale; a freshly issued id
    // is accepted again.
    requests.invalidate(RequestFamily::Entries);
    QVERIFY(!requests.accepts(RequestFamily::Entries, entriesId));
    const quint64 newEntriesId = requests.issue(RequestFamily::Entries);
    QVERIFY(requests.accepts(RequestFamily::Entries, newEntriesId));
    QVERIFY(!requests.accepts(RequestFamily::Entries, entriesId));

    // Every other family's outstanding request is untouched by the
    // entries churn.
    const quint64 inspectorId = requests.issue(RequestFamily::Inspector);
    const quint64 selectionId = requests.issue(RequestFamily::Selection);
    const quint64 candidatesId = requests.issue(RequestFamily::HardwareCandidates);
    const quint64 extractionId = requests.issue(RequestFamily::Extraction);
    requests.invalidate(RequestFamily::Entries);
    QVERIFY(requests.accepts(RequestFamily::Inspector, inspectorId));
    QVERIFY(requests.accepts(RequestFamily::Selection, selectionId));
    QVERIFY(requests.accepts(RequestFamily::HardwareCandidates, candidatesId));
    QVERIFY(requests.accepts(RequestFamily::Extraction, extractionId));

    // And the families really are five separate counters.
    QCOMPARE(requests.current(RequestFamily::Inspector), 1);
    QCOMPARE(requests.current(RequestFamily::Selection), 1);
    QCOMPARE(requests.current(RequestFamily::HardwareCandidates), 1);
    QCOMPARE(requests.current(RequestFamily::Extraction), 1);
    QCOMPARE(requests.current(RequestFamily::Entries), 4);
}

void SessionStateTest::failedReopenPreservesSession()
{
    SessionState session;
    driveToCommittedSession(session);

    session.beginOpen();
    QCOMPARE(session.openPhase(), OpenPhase::Opening);
    // Opening is not "no distribution": the committed facts stay live.
    QVERIFY(session.hasDistribution());
    session.openFailed();

    // Everything committed or derived survived; only the phase moved.
    QCOMPARE(session.openPhase(), OpenPhase::Idle);
    QVERIFY(session.hasDistribution());
    QCOMPARE(session.committedDistribution()->productCount, 2);
    QCOMPARE(session.committedDistribution()->diagnosticCount, 4);
    QCOMPARE(session.profile().size(), 1);
    QCOMPARE(session.profile().first().value, QStringLiteral("IP22"));
    QCOMPARE(session.candidates().size(), 1);
    QCOMPARE(session.selectionPhase(), SelectionPhase::Ready);
    QCOMPARE(session.selectedRecordCount(), 6);
    QCOMPARE(session.conflictGroupCount(), 1);
    QVERIFY(session.selectionError().isEmpty());
    QVERIFY(session.searchActive());
}

void SessionStateTest::rejectedCandidatePreservesSession()
{
    SessionState session;
    driveToCommittedSession(session);

    session.beginOpen();
    session.rejectCandidate();

    QCOMPARE(session.openPhase(), OpenPhase::Idle);
    QCOMPARE(session.displayedDistributionSummary()->productCount, 2);
    QVERIFY(session.hasDistribution());
    QCOMPARE(session.committedDistribution()->productCount, 2);
    QCOMPARE(session.candidates().size(), 1);
    QCOMPARE(session.selectionPhase(), SelectionPhase::Ready);
    QVERIFY(session.searchActive());
}

void SessionStateTest::acceptedReplacementStagesPending()
{
    SessionState session;
    driveToCommittedSession(session);

    session.beginOpen();
    session.acceptCandidate({1, 2});

    // The transaction sits at Committing: the old distribution is
    // still the committed one, the new one is only staged.
    QCOMPARE(session.openPhase(), OpenPhase::Committing);
    QVERIFY(session.hasDistribution());
    QCOMPARE(session.committedDistribution()->productCount, 2);
    QCOMPARE(session.committedDistribution()->diagnosticCount, 4);
    // Presentation may already show the pending summary.
    QCOMPARE(session.displayedDistributionSummary()->productCount, 1);
    QCOMPARE(session.displayedDistributionSummary()->diagnosticCount, 2);

    // The profile survives; everything derived from the old
    // distribution died with it.
    QCOMPARE(session.profile().size(), 1);
    QCOMPARE(session.profile().first().value, QStringLiteral("IP22"));
    QCOMPARE(session.browseMode(), BrowseMode::Hierarchy);
    QVERIFY(!session.searchActive());
    QVERIFY(session.candidates().isEmpty());
    QCOMPARE(session.selectedRecordCount(), 0);
    QCOMPARE(session.conflictGroupCount(), 0);
    QVERIFY(session.selectionError().isEmpty());
    QCOMPARE(session.selectionPhase(), SelectionPhase::Pending);
}

void SessionStateTest::commitPublishesPending()
{
    SessionState session;
    driveToCommittedSession(session);

    session.beginOpen();
    session.acceptCandidate({1, 2});
    session.candidateCommitted();

    QCOMPARE(session.openPhase(), OpenPhase::Idle);
    QVERIFY(session.hasDistribution());
    QCOMPARE(session.committedDistribution()->productCount, 1);
    QCOMPARE(session.committedDistribution()->diagnosticCount, 2);
    QCOMPARE(session.displayedDistributionSummary()->productCount, 1);
}

void SessionStateTest::emptyProfileReplacementStaysInactive()
{
    SessionState session;
    session.beginOpen();
    session.acceptCandidate({2, 4});
    session.candidateCommitted();

    session.beginOpen();
    session.acceptCandidate({1, 2});

    // No profile: no selection restart, the phase stays Inactive.
    QCOMPARE(session.selectionPhase(), SelectionPhase::Inactive);
    session.candidateCommitted();
    QCOMPARE(session.selectionPhase(), SelectionPhase::Inactive);
}

void SessionStateTest::profileLifecycle()
{
    SessionState session;

    // A profile applied before any distribution is stored, not
    // evaluated.
    session.applyProfile(ip22Profile());
    QCOMPARE(session.profile().size(), 1);
    QCOMPARE(session.selectionPhase(), SelectionPhase::Inactive);

    // Committing a distribution with a stored profile restarts the
    // selection at Pending.
    session.beginOpen();
    session.acceptCandidate({2, 4});
    QCOMPARE(session.selectionPhase(), SelectionPhase::Pending);
    session.candidateCommitted();

    // The selection resolves: counts land, no error.
    session.selectionSucceeded(6, 1);
    QCOMPARE(session.selectionPhase(), SelectionPhase::Ready);
    QCOMPARE(session.selectedRecordCount(), 6);
    QCOMPARE(session.conflictGroupCount(), 1);
    QVERIFY(session.selectionError().isEmpty());

    // Switching profiles resets the derived selection and restarts at
    // Pending; the candidates of the distribution survive.
    session.setCandidates(someCandidates());
    session.applyProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP32")}});
    QCOMPARE(session.selectionPhase(), SelectionPhase::Pending);
    QCOMPARE(session.selectedRecordCount(), 0);
    QCOMPARE(session.conflictGroupCount(), 0);
    QCOMPARE(session.candidates().size(), 1);

    // A failing selection keeps the profile and records the error.
    session.selectionFailed(QStringLiteral("boom"));
    QCOMPARE(session.selectionPhase(), SelectionPhase::Error);
    QCOMPARE(session.selectionError(), QStringLiteral("boom"));
    QCOMPARE(session.profile().size(), 1);
    QCOMPARE(session.selectedRecordCount(), 0);

    // Clearing the profile disables the overlay wholesale.
    session.applyProfile({});
    QCOMPARE(session.selectionPhase(), SelectionPhase::Inactive);
    QVERIFY(session.selectionError().isEmpty());
    QVERIFY(session.profile().isEmpty());

    // A fresh profile on a committed distribution restarts at Pending.
    session.applyProfile(ip22Profile());
    QCOMPARE(session.selectionPhase(), SelectionPhase::Pending);
}

void SessionStateTest::extractionLifecycle()
{
    SessionState session;
    session.beginOpen();
    session.acceptCandidate({2, 4});
    session.candidateCommitted();

    QVERIFY(!session.extractionRunning());
    session.beginExtraction();
    QVERIFY(session.extractionRunning());
    session.endExtraction();
    QVERIFY(!session.extractionRunning());

    // An extraction may only ever start from Idle — never straddling
    // an open transaction (Q_ASSERT-guarded). The observable
    // precondition: while Opening or Committing the phase is not Idle,
    // and MainWindow's command layer refuses to begin there.
    session.beginOpen();
    QVERIFY(session.openPhase() != OpenPhase::Idle);
    session.acceptCandidate({1, 2});
    QVERIFY(session.openPhase() != OpenPhase::Idle);
    QVERIFY(!session.extractionRunning());
    session.candidateCommitted();
    QCOMPARE(session.openPhase(), OpenPhase::Idle);
}

QTEST_GUILESS_MAIN(SessionStateTest)

#include "SessionStateTest.moc"
