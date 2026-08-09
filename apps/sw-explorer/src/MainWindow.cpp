#include "MainWindow.h"

#include "BackendWorker.h"
#include "EntryBrowserWidget.h"
#include "InspectorWidget.h"
#include "models/DistributionTreeModel.h"

#include <QAction>
#include <QFileDialog>
#include <QHeaderView>
#include <QItemSelectionModel>
#include <QKeySequence>
#include <QLineEdit>
#include <QMenuBar>
#include <QMessageBox>
#include <QSignalBlocker>
#include <QSizePolicy>
#include <QSplitter>
#include <QStatusBar>
#include <QThread>
#include <QTimer>
#include <QToolBar>
#include <QTreeView>

MainWindow::MainWindow()
{
    setWindowTitle(QStringLiteral("SW Explorer"));
    resize(1200, 700);

    m_openAction = new QAction(tr("Open Distribution..."), this);
    connect(m_openAction, &QAction::triggered, this, &MainWindow::chooseDistribution);
    menuBar()->addMenu(tr("&File"))->addAction(m_openAction);

    // The toolbar carries the same open action as the menu on the
    // left and the global path search on the right.
    auto *toolbar = addToolBar(tr("Main"));
    toolbar->setObjectName(QStringLiteral("mainToolBar"));
    toolbar->setMovable(false);
    toolbar->addAction(m_openAction);

    auto *spacer = new QWidget(toolbar);
    spacer->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    toolbar->addWidget(spacer);

    m_searchEdit = new QLineEdit(toolbar);
    m_searchEdit->setObjectName(QStringLiteral("searchEdit"));
    m_searchEdit->setPlaceholderText(tr("Search paths..."));
    m_searchEdit->setClearButtonEnabled(true);
    m_searchEdit->setFixedWidth(280);
    // No distribution yet: nothing to search.
    m_searchEdit->setEnabled(false);
    toolbar->addWidget(m_searchEdit);

    // The standard Find shortcut focuses the search box.
    auto *findAction = new QAction(tr("Find"), this);
    findAction->setObjectName(QStringLiteral("findAction"));
    findAction->setShortcut(QKeySequence::Find);
    connect(findAction, &QAction::triggered, this, [this] {
        if (m_searchEdit->isEnabled()) {
            m_searchEdit->setFocus(Qt::ShortcutFocusReason);
            m_searchEdit->selectAll();
        }
    });
    addAction(findAction);

    // Keystrokes restart the debounce; only a pause (or Enter) turns
    // the current text into one backend request.
    m_searchDebounce = new QTimer(this);
    m_searchDebounce->setSingleShot(true);
    m_searchDebounce->setInterval(250);
    connect(m_searchDebounce, &QTimer::timeout, this, [this] {
        startSearch(m_searchEdit->text());
    });
    connect(m_searchEdit, &QLineEdit::textChanged, this, &MainWindow::onSearchTextChanged);
    connect(m_searchEdit, &QLineEdit::returnPressed, this, &MainWindow::onSearchReturnPressed);

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
    connect(m_treeView, &QTreeView::clicked, this, &MainWindow::onTreeClicked);
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
            &MainWindow::searchEntriesRequested,
            m_worker,
            &BackendWorker::searchEntriesRequested);
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
    //
    // Only now — with the candidate validated and accepted — is the
    // old distribution's search state cleared; a failed or rejected
    // open never touches it.
    exitSearch();
    m_treeView->setEnabled(false);
    m_searchEdit->setEnabled(false);
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
    m_searchEdit->setEnabled(true);
}

void MainWindow::onTreeSelectionChanged(const QModelIndex &current, const QModelIndex &previous)
{
    Q_UNUSED(previous);
    if (!current.isValid()) {
        clearSelection();
        return;
    }
    // Picking a hierarchy scope always leaves search mode: tree scope
    // and global search results never mix in the entry browser.
    exitSearch();
    activateHierarchySelection(current);
}

void MainWindow::onTreeClicked(const QModelIndex &index)
{
    // currentChanged does not fire when the clicked row already is
    // the current one; during a search that click must still leave
    // search mode and restore the scope. A click on any other row
    // arrives together with currentChanged, which handles it — the
    // guard keeps the two paths from issuing duplicate requests.
    if (!m_searchActive || index != m_treeView->currentIndex()) {
        return;
    }
    exitSearch();
    activateHierarchySelection(index);
}

void MainWindow::activateHierarchySelection(const QModelIndex &index)
{
    // QModelIndex -> HierarchyItem -> object identity: the selection
    // chain from view to backend object id and kind.
    const quint64 objectId = m_model->objectId(index);
    const HierarchyKind kind = m_model->kind(index);

    // A hierarchy selection drives both panes at once: the inspector
    // shows the object detail, the entry browser lists the scope's
    // IDB records. Each family gets its own fresh request id.
    ++m_activeInspectorRequestId;
    m_inspector->showLoading();
    emit detailRequested(m_activeInspectorRequestId, objectId, kind);

    ++m_activeEntriesRequestId;
    m_entries->showLoading(
        m_model->data(m_model->index(index.row(), DistributionTreeModel::NameColumn, index.parent()))
            .toString());
    emit entriesRequested(m_activeEntriesRequestId, objectId);
}

void MainWindow::onSearchTextChanged(const QString &text)
{
    if (text.isEmpty()) {
        if (!m_searchActive) {
            return;
        }
        // An empty field means browsing mode, never a "match
        // everything" search: drop the pending search and fall back
        // to the scope the tree still has selected.
        m_searchActive = false;
        m_searchDebounce->stop();
        restoreHierarchyScope();
        return;
    }

    // Entering or refining search mode. Every change invalidates both
    // request families, so a late response of an older query can
    // never flash its results; the debounce turns the keystrokes into
    // one request once the user pauses.
    m_searchActive = true;
    ++m_activeEntriesRequestId;
    ++m_activeInspectorRequestId;
    m_inspector->showEmpty();
    m_entries->showLoading(tr("Search: %1").arg(text));
    m_searchDebounce->start();
}

void MainWindow::onSearchReturnPressed()
{
    // Enter searches immediately with a fresh request id; the pending
    // debounce is stopped so it cannot fire a duplicate afterwards.
    m_searchDebounce->stop();
    const QString text = m_searchEdit->text();
    if (text.isEmpty()) {
        return;
    }
    startSearch(text);
}

void MainWindow::startSearch(const QString &query)
{
    if (query.isEmpty() || !m_searchActive) {
        return;
    }
    ++m_activeEntriesRequestId;
    emit searchEntriesRequested(m_activeEntriesRequestId, query);
}

void MainWindow::exitSearch()
{
    if (!m_searchActive && m_searchEdit->text().isEmpty()) {
        return;
    }
    // Programmatic clears must not re-enter the search state machine
    // through textChanged.
    const QSignalBlocker blocker(m_searchEdit);
    m_searchEdit->clear();
    m_searchDebounce->stop();
    m_searchActive = false;
}

void MainWindow::restoreHierarchyScope()
{
    const QModelIndex current = m_treeView->currentIndex();
    if (current.isValid()) {
        activateHierarchySelection(current);
    } else {
        clearSelection();
    }
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
    // No search while an open is in flight; a failed open hands the
    // previous distribution's search box back.
    m_searchEdit->setEnabled(!inProgress && m_hasDistribution);
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
        // A scope or query the user has already left: drop it
        // silently.
        return;
    }
    m_entries->showEntries(entries);
    // A successful render also clears any earlier entries or search
    // error from the status bar, back to the distribution state.
    restoreLoadedStatus();
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
