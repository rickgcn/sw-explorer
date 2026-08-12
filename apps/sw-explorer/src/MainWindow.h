#pragma once

#include "EntrySnapshot.h"
#include "ExtractionSnapshot.h"
#include "HardwareSnapshot.h"
#include "HierarchySnapshot.h"
#include "InspectorSnapshot.h"
#include "SessionState.h"

#include <QMainWindow>

QT_BEGIN_NAMESPACE
class QAction;
class QLineEdit;
class QModelIndex;
class QThread;
class QTimer;
class QToolButton;
class QTreeView;
QT_END_NAMESPACE

class BackendWorker;
class DistributionTreeModel;
class EntryBrowserWidget;
class ExtractionDialog;
class InspectorWidget;

// Main window: the distribution hierarchy tree on the left, the
// entry browser in the middle and the inspector on the right, split
// by a QSplitter. All backend work runs on the BackendWorker thread.
//
// The ownership split:
//
//   BackendWorker owns backend execution state.
//   SessionState owns GUI session facts and request generations.
//   MainWindow owns widgets, user actions, backend signal routing,
//   and rendering SessionState into the UI.
//
// SessionState is an explicit value state machine: the open
// transaction phases (Idle/Opening/Committing), the committed and
// pending distribution summaries, the browse mode, the hardware
// profile/selection lifecycle, the extraction lock and the five
// request generations. Only the response matching the newest request
// of its family is accepted, everything older is dropped silently.
//
// Top-level interaction availability is computed in exactly one
// place, refreshUiState(), from session facts plus widget facts; the
// persistent status bar text likewise in refreshPersistentStatus().
//
// The toolbar search box switches the entry browser between two
// exclusive content sources: a non-empty query is a global path
// search across the whole distribution, an empty query is hierarchy
// browsing of the selected tree scope. Both source kinds share the
// entries request family, and every query change invalidates the
// in-flight requests of both families.
//
// The hardware profile describes the simulated target machine. It is
// a pure overlay over the entry list: applying or changing a profile
// only triggers a hardware selection request, never an entries,
// search or detail reload. The profile survives opening another
// distribution; the selection results and the candidate suggestions
// do not.
//
// Extraction goes through the ExtractionDialog: its scope is exactly
// the entry keys the Files view displays (never a re-queried search
// or hierarchy scope), preflight runs as an async backend request,
// and the actual extraction re-plans inside the backend before
// anything is written. While an extraction runs, every other
// interaction is locked.
class MainWindow : public QMainWindow
{
    Q_OBJECT

public:
    MainWindow();
    ~MainWindow() override;

signals:
    // Routed 1:1 onto the BackendWorker; emitted only by the
    // openDistribution() command, never connected back into the GUI.
    void backendOpenRequested(const QString &path);
    void candidateAccepted();
    void candidateRejected();
    void detailRequested(quint64 requestId, quint64 objectId, HierarchyKind kind);
    void entriesRequested(quint64 requestId, quint64 scopeId);
    void searchEntriesRequested(quint64 requestId, const QString &query);
    void entryDetailRequested(quint64 requestId, quint64 productId, quint64 entryId);
    void hardwareCandidatesRequested(quint64 requestId);
    void selectionRequested(quint64 requestId, const HardwareProfileSnapshot &profile);
    void planExtractionRequested(quint64 requestId, const ExtractionRequestSnapshot &request);
    void extractEntriesRequested(quint64 requestId, const ExtractionRequestSnapshot &request);

public slots:
    // The only programmatic open command. Owns the whole transaction
    // precondition — no nested candidate transactions while an open
    // or an extraction is in flight — starts the Opening phase and
    // emits the backend request. The file dialog only picks the path.
    void openDistribution(const QString &path);
    // Applies a hardware profile: stores it, refreshes the toolbar
    // summary and issues a selection request against the committed
    // distribution. An empty profile disables the hardware overlay.
    // With no distribution loaded the profile is simply stored and
    // evaluated once one is committed. Ignored while an open or an
    // extraction is in flight.
    void applyHardwareProfile(const HardwareProfileSnapshot &profile);

private slots:
    void chooseDistribution();
    void openHardwareProfileDialog();
    void openExtractionDialog();
    void onCandidateReady(quint64 productCount,
                          quint64 diagnosticCount,
                          const HierarchySnapshot &hierarchy);
    void onDistributionOpenFailed(const QString &message);
    void onCandidateCommitted();
    void onTreeSelectionChanged(const QModelIndex &current, const QModelIndex &previous);
    void onTreeClicked(const QModelIndex &index);
    void onSearchTextChanged(const QString &text);
    void onSearchReturnPressed();
    void onProductDetailReady(quint64 requestId, const ProductDetailSnapshot &detail);
    void onImageDetailReady(quint64 requestId, const ImageDetailSnapshot &detail);
    void onSubsystemDetailReady(quint64 requestId, const SubsystemDetailSnapshot &detail);
    void onDetailFailed(quint64 requestId, const QString &message);
    void onEntriesReady(quint64 requestId, const EntryListSnapshot &entries);
    void onEntriesFailed(quint64 requestId, const QString &message);
    void onEntryDetailReady(quint64 requestId, const EntryDetailSnapshot &detail);
    void onEntrySelected(quint64 productId, quint64 entryId);
    void onSelectionReady(quint64 requestId, const SelectionSnapshot &selection);
    void onSelectionFailed(quint64 requestId, const QString &message);
    void onHardwareCandidatesReady(quint64 requestId,
                                   const HardwareCandidatesSnapshot &candidates);
    void onHardwareCandidatesFailed(quint64 requestId, const QString &message);
    // The dialog's extraction signals, routed into the extraction
    // request family.
    void onExtractionPreflightRequested(const ExtractionRequestSnapshot &request);
    void onExtractionRequested(const ExtractionRequestSnapshot &request);
    void onExtractionInvalidated();
    // The worker's extraction responses, routed back to the dialog.
    void onExtractionPlanReady(quint64 requestId, const ExtractionPlanSnapshot &plan);
    void onExtractionPlanFailed(quint64 requestId, const QString &message);
    void onExtractionFinished(quint64 requestId, const ExtractionReportSnapshot &report);
    void onExtractionFailed(quint64 requestId, const QString &message);

private:
    // The single authority for top-level interaction availability:
    // Open, Extract, the tree, the Files pane, the search box and the
    // hardware button, computed from session facts plus the widget
    // fact EntryBrowserWidget::hasVisibleEntries().
    void refreshUiState();
    // The single authority for the persistent status bar text: the
    // displayed distribution summary plus the hardware selection
    // suffix, read from session facts only.
    void refreshPersistentStatus();
    // Empties the inspector and the Files pane and invalidates their
    // request families; never touches the hardware selection state.
    void clearBrowsePanes();
    // Issues the detail and entries requests for one hierarchy index.
    void activateHierarchySelection(const QModelIndex &index);
    // Leaves search mode without re-entering the textChanged handler:
    // clears the field with blocked signals and stops the debounce.
    void exitSearch();
    // Reloads the tree's current scope, or empties both panes when
    // the tree has no selection.
    void restoreHierarchyScope();
    // Emits one global search request with a fresh entries requestId.
    void startSearch(const QString &query);
    // Rebuilds the hardware button text and tooltip from the
    // currently applied profile.
    void updateHardwareButton();

    QAction *m_openAction = nullptr;
    QAction *m_extractAction = nullptr;
    QToolButton *m_hardwareButton = nullptr;

    QTreeView *m_treeView = nullptr;
    DistributionTreeModel *m_model = nullptr;
    EntryBrowserWidget *m_entries = nullptr;
    InspectorWidget *m_inspector = nullptr;

    QLineEdit *m_searchEdit = nullptr;
    QTimer *m_searchDebounce = nullptr;

    // Every GUI session fact and the request generations. Kept in
    // sync with the committed backend: a failed open or a rejected
    // snapshot leaves both sides showing the old distribution.
    SessionState m_session;

    // The live extraction dialog, while one is exec'd; widget
    // lifetime, not session state.
    ExtractionDialog *m_extractionDialog = nullptr;

    QThread *m_workerThread = nullptr;
    BackendWorker *m_worker = nullptr;
};
