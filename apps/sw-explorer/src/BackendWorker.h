#pragma once

#include <QObject>
#include <QString>

#include <optional>

#include "EntrySnapshot.h"
#include "HierarchySnapshot.h"
#include "InspectorSnapshot.h"
#include "sw_gui_bridge.h"

// Owns the Rust backend and runs every call into it. Lives in a
// dedicated QThread; the GUI thread never touches sw::Backend.
//
// Opening a distribution is a two-phase commit: the new distribution
// is loaded into a *candidate* backend and its hierarchy is sent to
// the GUI. Only when the GUI model has validated and accepted the
// snapshot does commitCandidate() swap the candidate in as the live
// backend. Until then the previously committed backend keeps serving
// every query, so a rejected snapshot can never desynchronize the GUI
// tree from the Rust-side object ids.
//
// Detail queries are answered only by the committed backend; the
// candidate is never queried. Every detailRequested carries a
// requestId that the response echoes back unchanged, so the GUI can
// drop responses that arrive after the selection moved on.
class BackendWorker : public QObject
{
    Q_OBJECT

public:
    explicit BackendWorker(QObject *parent = nullptr);

public slots:
    void openDistribution(const QString &path);
    void commitCandidate();
    void discardCandidate();
    void detailRequested(quint64 requestId, quint64 objectId, HierarchyKind kind);
    void entriesRequested(quint64 requestId, quint64 scopeId);
    void entryDetailRequested(quint64 requestId, quint64 productId, quint64 entryId);

signals:
    void candidateReady(quint64 productCount,
                        quint64 diagnosticCount,
                        const HierarchySnapshot &hierarchy);
    void distributionOpenFailed(const QString &message);
    // Emitted after the candidate has actually been swapped in as the
    // committed backend: from this point on the new object ids are
    // queryable.
    void candidateCommitted();

    void productDetailReady(quint64 requestId, const ProductDetailSnapshot &detail);
    void imageDetailReady(quint64 requestId, const ImageDetailSnapshot &detail);
    void subsystemDetailReady(quint64 requestId, const SubsystemDetailSnapshot &detail);
    void detailFailed(quint64 requestId, const QString &message);

    void entriesReady(quint64 requestId, const EntryListSnapshot &entries);
    void entriesFailed(quint64 requestId, const QString &message);
    // Entry detail failures reuse detailFailed: the requestId keeps
    // the two inspector request families apart on the GUI side.
    void entryDetailReady(quint64 requestId, const EntryDetailSnapshot &detail);

private:
    // The committed backend: what every future query talks to.
    rust::Box<sw::Backend> m_backend;
    // A fully opened distribution awaiting GUI validation. Never
    // queries; only ever moved into m_backend by commitCandidate().
    std::optional<rust::Box<sw::Backend>> m_candidate;
};
