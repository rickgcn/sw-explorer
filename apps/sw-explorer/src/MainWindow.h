#pragma once

#include "HierarchySnapshot.h"

#include <QMainWindow>

QT_BEGIN_NAMESPACE
class QAction;
class QLabel;
class QModelIndex;
class QThread;
class QTreeView;
QT_END_NAMESPACE

class BackendWorker;
class DistributionTreeModel;

// Main window: the distribution hierarchy tree on the left and the
// future content area on the right, split by a QSplitter. All backend
// work runs on the BackendWorker thread.
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

private slots:
    void chooseDistribution();
    void onCandidateReady(quint64 productCount,
                          quint64 diagnosticCount,
                          const HierarchySnapshot &hierarchy);
    void onDistributionOpenFailed(const QString &message);
    void onTreeSelectionChanged(const QModelIndex &current, const QModelIndex &previous);

private:
    void setOpenInProgress(bool inProgress);
    void clearSelection();

    QAction *m_openAction = nullptr;

    QTreeView *m_treeView = nullptr;
    DistributionTreeModel *m_model = nullptr;
    QLabel *m_identityLabel = nullptr;
    QLabel *m_kindLabel = nullptr;

    // Last successfully loaded state, kept in sync with the committed
    // backend: a failed open or a rejected snapshot must leave both
    // sides showing the old distribution.
    bool m_hasDistribution = false;
    QString m_loadedStatusText;

    QThread *m_workerThread = nullptr;
    BackendWorker *m_worker = nullptr;
};
