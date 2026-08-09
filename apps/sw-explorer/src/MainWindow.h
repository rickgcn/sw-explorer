#pragma once

#include "HierarchySnapshot.h"
#include "InspectorSnapshot.h"

#include <QMainWindow>

QT_BEGIN_NAMESPACE
class QAction;
class QModelIndex;
class QThread;
class QTreeView;
QT_END_NAMESPACE

class BackendWorker;
class DistributionTreeModel;
class InspectorWidget;

// Main window: the distribution hierarchy tree on the left and the
// inspector on the right, split by a QSplitter. All backend work runs
// on the BackendWorker thread.
//
// Inspector requests carry a monotonically increasing requestId; only
// the response matching the newest request is accepted, everything
// older is dropped silently. Selection changes, model resets and
// newly opened distributions all invalidate the pending request.
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

private slots:
    void chooseDistribution();
    void onCandidateReady(quint64 productCount,
                          quint64 diagnosticCount,
                          const HierarchySnapshot &hierarchy);
    void onDistributionOpenFailed(const QString &message);
    void onCandidateCommitted();
    void onTreeSelectionChanged(const QModelIndex &current, const QModelIndex &previous);
    void onProductDetailReady(quint64 requestId, const ProductDetailSnapshot &detail);
    void onImageDetailReady(quint64 requestId, const ImageDetailSnapshot &detail);
    void onSubsystemDetailReady(quint64 requestId, const SubsystemDetailSnapshot &detail);
    void onDetailFailed(quint64 requestId, const QString &message);

private:
    void setOpenInProgress(bool inProgress);
    void clearSelection();
    void restoreLoadedStatus();

    QAction *m_openAction = nullptr;

    QTreeView *m_treeView = nullptr;
    DistributionTreeModel *m_model = nullptr;
    InspectorWidget *m_inspector = nullptr;

    // The newest inspector request; responses with any other id are
    // stale and dropped.
    quint64 m_activeRequestId = 0;

    // Last successfully loaded state, kept in sync with the committed
    // backend: a failed open or a rejected snapshot must leave both
    // sides showing the old distribution.
    bool m_hasDistribution = false;
    QString m_loadedStatusText;

    QThread *m_workerThread = nullptr;
    BackendWorker *m_worker = nullptr;
};
