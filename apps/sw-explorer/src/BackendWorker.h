#pragma once

#include <QObject>
#include <QString>

#include <optional>

#include "HierarchySnapshot.h"
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
class BackendWorker : public QObject
{
    Q_OBJECT

public:
    explicit BackendWorker(QObject *parent = nullptr);

public slots:
    void openDistribution(const QString &path);
    void commitCandidate();
    void discardCandidate();

signals:
    void candidateReady(quint64 productCount,
                        quint64 diagnosticCount,
                        const HierarchySnapshot &hierarchy);
    void distributionOpenFailed(const QString &message);

private:
    // The committed backend: what every future query talks to.
    rust::Box<sw::Backend> m_backend;
    // A fully opened distribution awaiting GUI validation. Never
    // queries; only ever moved into m_backend by commitCandidate().
    std::optional<rust::Box<sw::Backend>> m_candidate;
};
