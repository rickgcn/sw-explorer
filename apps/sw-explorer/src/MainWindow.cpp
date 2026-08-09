#include "MainWindow.h"

#include "BackendWorker.h"
#include "EntryBrowserWidget.h"
#include "InspectorWidget.h"
#include "models/DistributionTreeModel.h"

#include <QAction>
#include <QFileDialog>
#include <QHeaderView>
#include <QItemSelectionModel>
#include <QMenuBar>
#include <QMessageBox>
#include <QSplitter>
#include <QStatusBar>
#include <QThread>
#include <QTreeView>

MainWindow::MainWindow()
{
    setWindowTitle(QStringLiteral("SW Explorer"));
    resize(1200, 700);

    m_openAction = new QAction(tr("Open Distribution..."), this);
    connect(m_openAction, &QAction::triggered, this, &MainWindow::chooseDistribution);
    menuBar()->addMenu(tr("&File"))->addAction(m_openAction);

    // Left: the distribution hierarchy. Middle: the entries of the
    // selected scope. Right: the inspector.
    m_model = new DistributionTreeModel(this);
    m_treeView = new QTreeView;
    m_treeView->setModel(m_model);
    m_treeView->setUniformRowHeights(true);
    m_treeView->header()->setSectionResizeMode(DistributionTreeModel::NameColumn,
                                               QHeaderView::Stretch);
    m_treeView->header()->setSectionResizeMode(DistributionTreeModel::EntriesColumn,
                                               QHeaderView::ResizeToContents);

    m_entries = new EntryBrowserWidget;
    m_inspector = new InspectorWidget;

    auto *splitter = new QSplitter(this);
    splitter->addWidget(m_treeView);
    splitter->addWidget(m_entries);
    splitter->addWidget(m_inspector);
    splitter->setStretchFactor(0, 0);
    splitter->setStretchFactor(1, 1);
    splitter->setStretchFactor(2, 0);
    splitter->setSizes({300, 540, 360});
    setCentralWidget(splitter);

    connect(m_treeView->selectionModel(),
            &QItemSelectionModel::currentChanged,
            this,
            &MainWindow::onTreeSelectionChanged);
    connect(m_model, &QAbstractItemModel::modelReset, this, &MainWindow::clearSelection);
    connect(m_entries, &EntryBrowserWidget::entrySelected, this, &MainWindow::onEntrySelected);

    statusBar()->showMessage(tr("No distribution loaded."));

    // Backend worker thread: the Rust backend is owned and used only
    // by the worker; results come back as queued signals.
    m_workerThread = new QThread(this);
    m_worker = new BackendWorker;
    m_worker->moveToThread(m_workerThread);
    connect(m_workerThread, &QThread::finished, m_worker, &QObject::deleteLater);
    connect(this,
            &MainWindow::openDistributionRequested,
            m_worker,
            &BackendWorker::openDistribution);
    connect(this,
            &MainWindow::candidateAccepted,
            m_worker,
            &BackendWorker::commitCandidate);
    connect(this,
            &MainWindow::candidateRejected,
            m_worker,
            &BackendWorker::discardCandidate);
    connect(this,
            &MainWindow::detailRequested,
            m_worker,
            &BackendWorker::detailRequested);
    connect(this,
            &MainWindow::entriesRequested,
            m_worker,
            &BackendWorker::entriesRequested);
    connect(this,
            &MainWindow::entryDetailRequested,
            m_worker,
            &BackendWorker::entryDetailRequested);
    connect(m_worker, &BackendWorker::candidateReady, this, &MainWindow::onCandidateReady);
    connect(m_worker,
            &BackendWorker::distributionOpenFailed,
            this,
            &MainWindow::onDistributionOpenFailed);
    connect(m_worker,
            &BackendWorker::candidateCommitted,
            this,
            &MainWindow::onCandidateCommitted);
    connect(m_worker,
            &BackendWorker::productDetailReady,
            this,
            &MainWindow::onProductDetailReady);
    connect(m_worker,
            &BackendWorker::imageDetailReady,
            this,
            &MainWindow::onImageDetailReady);
    connect(m_worker,
            &BackendWorker::subsystemDetailReady,
            this,
            &MainWindow::onSubsystemDetailReady);
    connect(m_worker, &BackendWorker::detailFailed, this, &MainWindow::onDetailFailed);
    connect(m_worker, &BackendWorker::entriesReady, this, &MainWindow::onEntriesReady);
    connect(m_worker, &BackendWorker::entriesFailed, this, &MainWindow::onEntriesFailed);
    connect(m_worker,
            &BackendWorker::entryDetailReady,
            this,
            &MainWindow::onEntryDetailReady);
    m_workerThread->start();
}

MainWindow::~MainWindow()
{
    m_workerThread->quit();
    m_workerThread->wait();
}

void MainWindow::chooseDistribution()
{
    const QString path = QFileDialog::getExistingDirectory(this, tr("Open Distribution"));
    if (path.isEmpty()) {
        return;
    }

    setOpenInProgress(true);
    statusBar()->showMessage(tr("Loading..."));
    emit openDistributionRequested(path);
}

void MainWindow::onCandidateReady(quint64 productCount,
                                  quint64 diagnosticCount,
                                  const HierarchySnapshot &hierarchy)
{
    setOpenInProgress(false);

    QString error;
    if (!m_model->setHierarchy(hierarchy, &error)) {
        // The snapshot violates the model's invariants. Reject the
        // candidate: the worker drops it and keeps the previously
        // committed backend, which still matches the tree on screen.
        emit candidateRejected();
        statusBar()->showMessage(m_hasDistribution ? m_loadedStatusText
                                                   : tr("No distribution loaded."));
        QMessageBox::critical(this,
                              tr("Failed to Open Distribution"),
                              tr("The backend returned an invalid hierarchy: %1").arg(error));
        return;
    }

    // The tree now shows the candidate; commit it so the backend
    // serves the very object ids the tree carries.
    //
    // The model reset above cleared the selection (and with it the
    // inspector). Between accepting the candidate and the worker's
    // candidateCommitted() the committed backend still holds the old
    // distribution while the tree already carries the new object ids,
    // so tree interaction is locked until the commit lands: no
    // inspector request may be issued in that window.
    m_treeView->setEnabled(false);
    emit candidateAccepted();

    QString text = tr("Loaded %1 products").arg(productCount);
    if (diagnosticCount > 0) {
        text += tr(" · %1 diagnostics").arg(diagnosticCount);
    }

    m_hasDistribution = true;
    m_loadedStatusText = text;
    statusBar()->showMessage(text);
}

void MainWindow::onDistributionOpenFailed(const QString &message)
{
    setOpenInProgress(false);
    // The backend kept the previously loaded distribution; the GUI
    // must keep showing it too.
    statusBar()->showMessage(m_hasDistribution ? m_loadedStatusText
                                               : tr("No distribution loaded."));
    QMessageBox::critical(this, tr("Failed to Open Distribution"), message);
}

void MainWindow::onCandidateCommitted()
{
    // The backend now serves the object ids the tree carries.
    m_treeView->setEnabled(true);
}

void MainWindow::onTreeSelectionChanged(const QModelIndex &current, const QModelIndex &previous)
{
    Q_UNUSED(previous);
    if (!current.isValid()) {
        clearSelection();
        return;
    }

    // QModelIndex -> HierarchyItem -> object identity: the selection
    // chain from view to backend object id and kind.
    const quint64 objectId = m_model->objectId(current);
    const HierarchyKind kind = m_model->kind(current);

    // A hierarchy selection drives both panes at once: the inspector
    // shows the object detail, the entry browser lists the scope's
    // IDB records. Each family gets its own fresh request id.
    ++m_activeInspectorRequestId;
    m_inspector->showLoading();
    emit detailRequested(m_activeInspectorRequestId, objectId, kind);

    ++m_activeEntriesRequestId;
    m_entries->showLoading(
        m_model->data(m_model->index(current.row(), DistributionTreeModel::NameColumn, current.parent()))
            .toString());
    emit entriesRequested(m_activeEntriesRequestId, objectId);
}

void MainWindow::clearSelection()
{
    // Invalidate the pending requests: a late response must never
    // overwrite the cleared panes.
    ++m_activeInspectorRequestId;
    ++m_activeEntriesRequestId;
    m_inspector->showEmpty();
    m_entries->showEmpty();
}

void MainWindow::onEntrySelected(quint64 productId, quint64 entryId)
{
    // An entry pick replaces only the inspector content; the entry
    // list the user is browsing stays untouched.
    ++m_activeInspectorRequestId;
    m_inspector->showLoading();
    emit entryDetailRequested(m_activeInspectorRequestId, productId, entryId);
}

void MainWindow::setOpenInProgress(bool inProgress)
{
    m_openAction->setEnabled(!inProgress);
}

void MainWindow::onProductDetailReady(quint64 requestId, const ProductDetailSnapshot &detail)
{
    if (requestId != m_activeInspectorRequestId) {
        return;
    }
    m_inspector->showProduct(detail);
    restoreLoadedStatus();
}

void MainWindow::onImageDetailReady(quint64 requestId, const ImageDetailSnapshot &detail)
{
    if (requestId != m_activeInspectorRequestId) {
        return;
    }
    m_inspector->showImage(detail);
    restoreLoadedStatus();
}

void MainWindow::onSubsystemDetailReady(quint64 requestId, const SubsystemDetailSnapshot &detail)
{
    if (requestId != m_activeInspectorRequestId) {
        return;
    }
    m_inspector->showSubsystem(detail);
    restoreLoadedStatus();
}

void MainWindow::onDetailFailed(quint64 requestId, const QString &message)
{
    if (requestId != m_activeInspectorRequestId) {
        // Stale failure: the user has moved on; stay silent.
        return;
    }
    m_inspector->showError(message);
    statusBar()->showMessage(tr("Unable to load details: %1").arg(message));
}

void MainWindow::onEntryDetailReady(quint64 requestId, const EntryDetailSnapshot &detail)
{
    if (requestId != m_activeInspectorRequestId) {
        return;
    }
    m_inspector->showEntry(detail);
    restoreLoadedStatus();
}

void MainWindow::onEntriesReady(quint64 requestId, const EntryListSnapshot &entries)
{
    if (requestId != m_activeEntriesRequestId) {
        // A scope the user has already left: drop it silently.
        return;
    }
    m_entries->showEntries(entries);
}

void MainWindow::onEntriesFailed(quint64 requestId, const QString &message)
{
    if (requestId != m_activeEntriesRequestId) {
        return;
    }
    m_entries->showError(message);
    statusBar()->showMessage(tr("Unable to load entries: %1").arg(message));
}

void MainWindow::restoreLoadedStatus()
{
    // A successful detail render also clears any earlier detail error
    // from the status bar, back to the normal distribution state.
    statusBar()->showMessage(m_hasDistribution ? m_loadedStatusText
                                               : tr("No distribution loaded."));
}
