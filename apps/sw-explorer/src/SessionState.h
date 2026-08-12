#pragma once

#include "HardwareSnapshot.h"

#include <QString>
#include <QtGlobal>

#include <array>
#include <cstddef>
#include <optional>

// The GUI session state machine: an explicit value type holding every
// session fact MainWindow used to scatter across loose members.
//
// SessionState is not a QObject, emits no signals, owns no widgets and
// no BackendWorker, and knows nothing about Rust/CXX. It is pure
// GUI-thread value state: MainWindow translates user actions and
// backend responses into transitions and renders the facts back into
// the UI.
//
// The open transaction mirrors the backend's candidate/commit
// protocol: beginOpen() leaves the committed distribution untouched,
// acceptCandidate() stages the new distribution as pending and clears
// every fact derived from the old one, candidateCommitted() publishes
// pending as committed. A failed or rejected open keeps the committed
// distribution and every derived fact exactly as they were.

// The open transaction phase. Idle covers both "no distribution yet"
// and "committed distribution, no open in flight"; Opening and
// Committing can never overlap.
enum class OpenPhase {
    // No candidate transaction is running.
    Idle,
    // The backend is loading a candidate distribution.
    Opening,
    // The candidate hierarchy was validated and accepted; the backend
    // commit has not landed yet.
    Committing,
};

// Which content source the entry browser shows. The query text itself
// stays in the search QLineEdit; only the mode is session state.
enum class BrowseMode {
    // The entry browser follows the tree selection.
    Hierarchy,
    // The entry browser shows global path search results.
    Search,
};

// The lifecycle of the hardware selection overlay.
enum class SelectionPhase {
    // No profile applied, or a profile waiting for its first
    // distribution.
    Inactive,
    // A selection request is required or in flight for the committed
    // distribution.
    Pending,
    // The latest selection succeeded.
    Ready,
    // The latest selection failed; the profile is kept.
    Error,
};

// The headline facts of one opened distribution.
struct DistributionSummary {
    quint64 productCount = 0;
    quint64 diagnosticCount = 0;
};

// The async request families. Each family has its own generation so a
// slow entry list never invalidates a quick entry detail, re-profiling
// never invalidates browsing, and extraction never collides with
// either.
enum class RequestFamily {
    Inspector,
    Entries,
    Selection,
    HardwareCandidates,
    Extraction,
    Count,
};

// One monotonically increasing generation counter per request family.
// Only the response carrying the family's current generation is
// accepted; anything older is stale and dropped.
class RequestGenerations
{
public:
    // Bumps the family's generation and returns it as the id of the
    // new request.
    quint64 issue(RequestFamily family)
    {
        return ++m_generations.at(indexOf(family));
    }

    // Bumps the family's generation without issuing a request: every
    // id issued so far becomes stale.
    void invalidate(RequestFamily family)
    {
        ++m_generations.at(indexOf(family));
    }

    // Whether `requestId` is the family's current generation.
    bool accepts(RequestFamily family, quint64 requestId) const
    {
        return requestId == m_generations.at(indexOf(family));
    }

    // The family's current generation.
    quint64 current(RequestFamily family) const
    {
        return m_generations.at(indexOf(family));
    }

private:
    static constexpr std::size_t indexOf(RequestFamily family)
    {
        return static_cast<std::size_t>(family);
    }

    std::array<quint64, static_cast<std::size_t>(RequestFamily::Count)> m_generations{};
};

// Every GUI session fact plus the request generations, with the legal
// transitions as methods. Illegal programmatic transitions are caught
// by Q_ASSERT; the public UI commands guard before calling in.
class SessionState
{
public:
    // --- open transaction ---

    OpenPhase openPhase() const
    {
        return m_openPhase;
    }

    // Only the committed backend counts: a pending candidate is never
    // treated as an opened distribution.
    bool hasDistribution() const
    {
        return m_committedDistribution.has_value();
    }

    // The summary of the committed backend.
    std::optional<DistributionSummary> committedDistribution() const
    {
        return m_committedDistribution;
    }

    // Presentation only: while a candidate waits for its commit, the
    // status bar may already show the pending summary. Capability
    // decisions must use hasDistribution(), never this.
    std::optional<DistributionSummary> displayedDistributionSummary() const
    {
        if (m_pendingDistribution.has_value()) {
            return m_pendingDistribution;
        }
        return m_committedDistribution;
    }

    // Idle -> Opening. The committed distribution and everything
    // derived from it stay untouched: the open may still fail.
    void beginOpen()
    {
        Q_ASSERT(m_openPhase == OpenPhase::Idle);
        Q_ASSERT(!m_extractionRunning);
        m_openPhase = OpenPhase::Opening;
    }

    // Opening -> Idle after the backend refused the candidate. Every
    // committed fact is preserved.
    void openFailed()
    {
        Q_ASSERT(m_openPhase == OpenPhase::Opening);
        m_openPhase = OpenPhase::Idle;
    }

    // Opening -> Idle after the GUI model rejected the candidate
    // hierarchy. Every committed fact is preserved.
    void rejectCandidate()
    {
        Q_ASSERT(m_openPhase == OpenPhase::Opening);
        m_pendingDistribution.reset();
        m_openPhase = OpenPhase::Idle;
    }

    // Opening -> Committing: the hierarchy snapshot validated. Stages
    // the new summary as pending and drops everything derived from the
    // old distribution — search mode, candidate suggestions, selection
    // results — while the hardware profile survives. A non-empty
    // profile restarts at Pending against the coming distribution.
    void acceptCandidate(DistributionSummary summary)
    {
        Q_ASSERT(m_openPhase == OpenPhase::Opening);
        m_pendingDistribution = summary;
        m_browseMode = BrowseMode::Hierarchy;
        m_candidates.clear();
        m_selectionError.clear();
        m_selectedRecordCount = 0;
        m_conflictGroupCount = 0;
        m_selectionPhase =
            m_profile.isEmpty() ? SelectionPhase::Inactive : SelectionPhase::Pending;
        m_openPhase = OpenPhase::Committing;
    }

    // Committing -> Idle: the backend swapped the candidate in, so the
    // pending summary becomes the committed one.
    void candidateCommitted()
    {
        Q_ASSERT(m_openPhase == OpenPhase::Committing);
        Q_ASSERT(m_pendingDistribution.has_value());
        m_committedDistribution = m_pendingDistribution;
        m_pendingDistribution.reset();
        m_openPhase = OpenPhase::Idle;
    }

    // --- browse mode ---

    BrowseMode browseMode() const
    {
        return m_browseMode;
    }

    bool searchActive() const
    {
        return m_browseMode == BrowseMode::Search;
    }

    void enterSearch()
    {
        m_browseMode = BrowseMode::Search;
    }

    void leaveSearch()
    {
        m_browseMode = BrowseMode::Hierarchy;
    }

    // --- hardware profile, candidates and selection ---

    // The applied profile: facts about the simulated target machine,
    // kept for the whole run, across distributions.
    const HardwareProfileSnapshot &profile() const
    {
        return m_profile;
    }

    // The value suggestions of the committed distribution.
    const HardwareCandidatesSnapshot &candidates() const
    {
        return m_candidates;
    }

    // Suggestions belong to the current distribution: replaced by a
    // fresh response, cleared only by a successful replacement.
    void setCandidates(const HardwareCandidatesSnapshot &candidates)
    {
        m_candidates = candidates;
    }

    SelectionPhase selectionPhase() const
    {
        return m_selectionPhase;
    }

    const QString &selectionError() const
    {
        return m_selectionError;
    }

    quint64 selectedRecordCount() const
    {
        return m_selectedRecordCount;
    }

    quint64 conflictGroupCount() const
    {
        return m_conflictGroupCount;
    }

    // Stores a new profile and resets the selection derived from the
    // old one: an empty profile, or no committed distribution, lands
    // at Inactive; a non-empty profile with a committed distribution
    // restarts at Pending.
    void applyProfile(const HardwareProfileSnapshot &profile)
    {
        m_profile = profile;
        m_selectionError.clear();
        m_selectedRecordCount = 0;
        m_conflictGroupCount = 0;
        if (m_profile.isEmpty() || !hasDistribution()) {
            m_selectionPhase = SelectionPhase::Inactive;
        } else {
            m_selectionPhase = SelectionPhase::Pending;
        }
    }

    void selectionSucceeded(quint64 selectedRecordCount, quint64 conflictGroupCount)
    {
        m_selectionPhase = SelectionPhase::Ready;
        m_selectedRecordCount = selectedRecordCount;
        m_conflictGroupCount = conflictGroupCount;
        m_selectionError.clear();
    }

    void selectionFailed(const QString &error)
    {
        m_selectionPhase = SelectionPhase::Error;
        m_selectionError = error;
        m_selectedRecordCount = 0;
        m_conflictGroupCount = 0;
    }

    // --- extraction ---

    // The session only tracks the global interaction lock; the dialog
    // owns its own finer-grained state machine.
    bool extractionRunning() const
    {
        return m_extractionRunning;
    }

    void beginExtraction()
    {
        Q_ASSERT(m_openPhase == OpenPhase::Idle);
        Q_ASSERT(!m_extractionRunning);
        m_extractionRunning = true;
    }

    void endExtraction()
    {
        Q_ASSERT(m_extractionRunning);
        m_extractionRunning = false;
    }

    // --- request generations ---

    RequestGenerations &requests()
    {
        return m_requests;
    }

    const RequestGenerations &requests() const
    {
        return m_requests;
    }

private:
    OpenPhase m_openPhase = OpenPhase::Idle;
    std::optional<DistributionSummary> m_committedDistribution;
    std::optional<DistributionSummary> m_pendingDistribution;
    BrowseMode m_browseMode = BrowseMode::Hierarchy;
    HardwareProfileSnapshot m_profile;
    HardwareCandidatesSnapshot m_candidates;
    SelectionPhase m_selectionPhase = SelectionPhase::Inactive;
    QString m_selectionError;
    quint64 m_selectedRecordCount = 0;
    quint64 m_conflictGroupCount = 0;
    bool m_extractionRunning = false;
    RequestGenerations m_requests;
};
