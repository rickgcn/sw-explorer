#pragma once

#include "EntrySnapshot.h"
#include "HierarchySnapshot.h"
#include "InspectorSnapshot.h"

#include <QMainWindow>

QT_BEGIN_NAMESPACE
class QAction;
class QLineEdit;
class QModelIndex;
class QThread;
class QTimer;
class QTreeView;
QT_END_NAMESPACE

class BackendWorker;
class DistributionTreeModel;
class EntryBrowserWidget;
class InspectorWidget;

// Main window: the distribution hierarchy tree on the left, the
// entry browser in the middle and the inspector on the right, split
// by a QSplitter. All backend work runs on the BackendWorker thread.
//
// Inspector and entry-list requests each carry their own
// monotonically increasing requestId; only the response matching the
// newest request of its family is accepted, everything older is
// dropped silently. Selection changes, model resets and newly opened
// distributions all invalidate the pending requests.
//
// The toolbar search box switches the entry browser between two
// exclusive content sources: a non-empty query is a global path
// search across the whole distribution, an empty query is hierarchy
// browsing of the selected tree scope. Both source kinds share the
// entries request family, and every query change invalidates the
// in-flight requests of both families.
class MainWindow : public QMainWindow
{
    Q_OBJECT

public:
    MainWindow();
    ~MainWindow() override;

signals:
    void openDistributionRequested(const QString &path);
    void candidateAccepted();
    void candidateRejected();
    void detailRequested(quint64 requestId, quint64 objectId, HierarchyKind kind);
    void entriesRequested(quint64 requestId, quint64 scopeId);
    void searchEntriesRequested(quint64 requestId, const QString &query);
    void entryDetailRequested(quint64 requestId, quint64 productId, quint64 entryId);

private slots:
    void chooseDistribution();
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

private:
    void setOpenInProgress(bool inProgress);
    void clearSelection();
    void restoreLoadedStatus();
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

    QAction *m_openAction = nullptr;

    QTreeView *m_treeView = nullptr;
    DistributionTreeModel *m_model = nullptr;
    EntryBrowserWidget *m_entries = nullptr;
    InspectorWidget *m_inspector = nullptr;

    QLineEdit *m_searchEdit = nullptr;
    QTimer *m_searchDebounce = nullptr;
    // Whether the entry browser shows search results rather than a
    // hierarchy scope; never inferred from the pane contents.
    bool m_searchActive = false;

    // The newest inspector request (object detail or entry detail);
    // responses with any other id are stale and dropped.
    quint64 m_activeInspectorRequestId = 0;
    // The newest entry-list request (scope listing or search
    // results); a separate family, so a slow entry list never
    // invalidates a quick entry detail.
    quint64 m_activeEntriesRequestId = 0;

    // Last successfully loaded state, kept in sync with the committed
    // backend: a failed open or a rejected snapshot must leave both
    // sides showing the old distribution.
    bool m_hasDistribution = false;
    QString m_loadedStatusText;

    QThread *m_workerThread = nullptr;
    BackendWorker *m_worker = nullptr;
};
