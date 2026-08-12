#include "MainWindow.h"

#include "BackendWorker.h"
#include "EntryBrowserWidget.h"
#include "ExtractionDialog.h"
#include "HardwareProfileDialog.h"
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
#include <QToolButton>
#include <QTreeView>

MainWindow::MainWindow()
{
    setWindowTitle(QStringLiteral("SW Explorer"));
    resize(1200, 700);

    m_openAction = new QAction(tr("Open Distribution..."), this);
    m_openAction->setObjectName(QStringLiteral("openAction"));
    connect(m_openAction, &QAction::triggered, this, &MainWindow::chooseDistribution);
    QMenu *fileMenu = menuBar()->addMenu(tr("&File"));
    fileMenu->addAction(m_openAction);

    // Extract shares the action between menu and toolbar; its
    // availability is computed in refreshUiState().
    m_extractAction = new QAction(tr("Extract..."), this);
    m_extractAction->setObjectName(QStringLiteral("extractAction"));
    connect(m_extractAction, &QAction::triggered, this, &MainWindow::openExtractionDialog);
    fileMenu->addAction(m_extractAction);

    // The toolbar carries the same open action as the menu on the
    // left and the global path search on the right.
    auto *toolbar = addToolBar(tr("Main"));
    toolbar->setObjectName(QStringLiteral("mainToolBar"));
    toolbar->setMovable(false);
    toolbar->addAction(m_openAction);
    toolbar->addAction(m_extractAction);

    // The hardware profile editor is always available, even with no
    // distribution loaded: the profile can be configured first and is
    // evaluated once a distribution is committed.
    m_hardwareButton = new QToolButton(toolbar);
    m_hardwareButton->setObjectName(QStringLiteral("hardwareButton"));
    connect(m_hardwareButton,
            &QToolButton::clicked,
            this,
            &MainWindow::openHardwareProfileDialog);
    toolbar->addWidget(m_hardwareButton);
    updateHardwareButton();

    auto *spacer = new QWidget(toolbar);
    spacer->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    toolbar->addWidget(spacer);

    m_searchEdit = new QLineEdit(toolbar);
    m_searchEdit->setObjectName(QStringLiteral("searchEdit"));
    m_searchEdit->setPlaceholderText(tr("Search paths..."));
    m_searchEdit->setClearButtonEnabled(true);
    m_searchEdit->setFixedWidth(280);
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
    connect(m_model, &QAbstractItemModel::modelReset, this, &MainWindow::clearBrowsePanes);
    connect(m_entries, &EntryBrowserWidget::entrySelected, this, &MainWindow::onEntrySelected);

    // Backend worker thread: the Rust backend is owned and used only
    // by the worker; results come back as queued signals.
    m_workerThread = new QThread(this);
    m_worker = new BackendWorker;
    m_worker->moveToThread(m_workerThread);
    connect(m_workerThread, &QThread::finished, m_worker, &QObject::deleteLater);
    connect(this,
            &MainWindow::backendOpenRequested,
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
    connect(this,
            &MainWindow::hardwareCandidatesRequested,
            m_worker,
            &BackendWorker::hardwareCandidatesRequested);
    connect(this,
            &MainWindow::selectionRequested,
            m_worker,
            &BackendWorker::selectionRequested);
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
    connect(m_worker,
            &BackendWorker::hardwareCandidatesReady,
            this,
            &MainWindow::onHardwareCandidatesReady);
    connect(m_worker,
            &BackendWorker::hardwareCandidatesFailed,
            this,
            &MainWindow::onHardwareCandidatesFailed);
    connect(m_worker, &BackendWorker::selectionReady, this, &MainWindow::onSelectionReady);
    connect(m_worker, &BackendWorker::selectionFailed, this, &MainWindow::onSelectionFailed);
    connect(this,
            &MainWindow::planExtractionRequested,
            m_worker,
            &BackendWorker::planExtractionRequested);
    connect(this,
            &MainWindow::extractEntriesRequested,
            m_worker,
            &BackendWorker::extractEntriesRequested);
    connect(m_worker,
            &BackendWorker::extractionPlanReady,
            this,
            &MainWindow::onExtractionPlanReady);
    connect(m_worker,
            &BackendWorker::extractionPlanFailed,
            this,
            &MainWindow::onExtractionPlanFailed);
    connect(m_worker,
            &BackendWorker::extractionFinished,
            this,
            &MainWindow::onExtractionFinished);
    connect(m_worker,
            &BackendWorker::extractionFailed,
            this,
            &MainWindow::onExtractionFailed);
    m_workerThread->start();

    refreshUiState();
    refreshPersistentStatus();
}

MainWindow::~MainWindow()
{
    m_workerThread->quit();
    m_workerThread->wait();
}

void MainWindow::refreshUiState()
{
    const bool idle = m_session.openPhase() == OpenPhase::Idle;
    const bool extracting = m_session.extractionRunning();
    const bool hasDistribution = m_session.hasDistribution();

    // The whole open transaction — Opening and Committing alike —
    // locks Open and the hardware profile, so no profile change or
    // nested open can straddle the candidate/commit window.
    m_openAction->setEnabled(idle && !extracting);
    m_hardwareButton->setEnabled(idle && !extracting);

    // The old committed hierarchy stays browsable while a candidate
    // loads; only the commit window and extractions lock the tree.
    m_treeView->setEnabled(m_session.openPhase() != OpenPhase::Committing && !extracting);
    m_entries->setEnabled(!extracting);

    // Searching needs a committed distribution and a quiet session:
    // no open transaction, no extraction.
    m_searchEdit->setEnabled(hasDistribution && idle && !extracting);

    // With a hardware profile the selection must be Ready before the
    // dialog opens; the planner re-evaluates it authoritatively anyway,
    // so this only keeps the user from writing while the UI itself
    // reports the selection as unresolved.
    const bool selectionReady =
        m_session.profile().isEmpty() || m_session.selectionPhase() == SelectionPhase::Ready;
    m_extractAction->setEnabled(hasDistribution && m_entries->hasVisibleEntries() && idle
                                && !extracting && selectionReady);
}

void MainWindow::refreshPersistentStatus()
{
    // The persistent line: the displayed distribution summary plus the
    // hardware selection state, both read from the session. Transient
    // messages (loading, detail/entries errors) overwrite it directly;
    // the next successful render restores it here.
    QString text;
    const std::optional<DistributionSummary> summary = m_session.displayedDistributionSummary();
    if (summary.has_value()) {
        text = tr("Loaded %1 products").arg(summary->productCount);
        if (summary->diagnosticCount > 0) {
            text += tr(" · %1 diagnostics").arg(summary->diagnosticCount);
        }
    } else {
        text = tr("No distribution loaded.");
    }
    switch (m_session.selectionPhase()) {
    case SelectionPhase::Inactive:
        break;
    case SelectionPhase::Pending:
        text += tr(" · Evaluating hardware profile...");
        break;
    case SelectionPhase::Ready:
        text += tr(" · %1 selected records · %2 conflict groups")
                    .arg(m_session.selectedRecordCount())
                    .arg(m_session.conflictGroupCount());
        break;
    case SelectionPhase::Error:
        text += tr(" · Hardware selection unavailable: %1").arg(m_session.selectionError());
        break;
    }
    statusBar()->showMessage(text);
}

void MainWindow::chooseDistribution()
{
    const QString path = QFileDialog::getExistingDirectory(this, tr("Open Distribution"));
    if (!path.isEmpty()) {
        openDistribution(path);
    }
}

void MainWindow::openDistribution(const QString &path)
{
    // Second-line state protection: the action is disabled across
    // open/commit/extraction windows, and a programmatic call must
    // not start a nested candidate transaction either.
    if (m_session.openPhase() != OpenPhase::Idle || m_session.extractionRunning()) {
        return;
    }
    m_session.beginOpen();
    refreshUiState();
    statusBar()->showMessage(tr("Loading..."));
    emit backendOpenRequested(path);
}

void MainWindow::openHardwareProfileDialog()
{
    HardwareProfileDialog dialog(this);
    dialog.setCandidates(m_session.candidates());
    dialog.setProfile(m_session.profile());
    connect(&dialog,
            &HardwareProfileDialog::profileApplied,
            this,
            &MainWindow::applyHardwareProfile);
    dialog.exec();
}

void MainWindow::openExtractionDialog()
{
    if (!m_extractAction->isEnabled()) {
        return;
    }
    ExtractionDialog dialog(this);
    m_extractionDialog = &dialog;
    // The scope is exactly what the Files view shows right now: its
    // entry keys, never a re-queried search text or hierarchy scope.
    const std::optional<EntryKey> selected = m_entries->currentEntryKey();
    QString selectedPath;
    if (selected.has_value()) {
        selectedPath = m_entries->currentEntryPath();
    }
    dialog.setScope(m_entries->entryKeys(), selected, selectedPath);
    dialog.setHardwareProfile(m_session.profile());
    connect(&dialog,
            &ExtractionDialog::preflightRequested,
            this,
            &MainWindow::onExtractionPreflightRequested);
    connect(&dialog,
            &ExtractionDialog::extractRequested,
            this,
            &MainWindow::onExtractionRequested);
    connect(&dialog,
            &ExtractionDialog::requestInvalidated,
            this,
            &MainWindow::onExtractionInvalidated);
    dialog.exec();
    // The dialog is gone: a preflight in flight was invalidated by its
    // reject, and a running extraction cannot be closed out of.
    m_extractionDialog = nullptr;
}

void MainWindow::onExtractionPreflightRequested(const ExtractionRequestSnapshot &request)
{
    // A preflight does not enter the global extraction lock: the
    // modal dialog's own Checking state covers the interaction.
    const quint64 requestId = m_session.requests().issue(RequestFamily::Extraction);
    emit planExtractionRequested(requestId, request);
}

void MainWindow::onExtractionRequested(const ExtractionRequestSnapshot &request)
{
    const quint64 requestId = m_session.requests().issue(RequestFamily::Extraction);
    m_session.beginExtraction();
    refreshUiState();
    emit extractEntriesRequested(requestId, request);
}

void MainWindow::onExtractionInvalidated()
{
    // The request changed after (or during) a preflight, or the dialog
    // was closed mid-check: anything in flight for it is stale.
    m_session.requests().invalidate(RequestFamily::Extraction);
}

void MainWindow::onExtractionPlanReady(quint64 requestId, const ExtractionPlanSnapshot &plan)
{
    if (!m_session.requests().accepts(RequestFamily::Extraction, requestId)
        || m_extractionDialog == nullptr) {
        // A preflight the user has already moved on from: dropped.
        return;
    }
    m_extractionDialog->showPlan(plan);
}

void MainWindow::onExtractionPlanFailed(quint64 requestId, const QString &message)
{
    if (!m_session.requests().accepts(RequestFamily::Extraction, requestId)
        || m_extractionDialog == nullptr) {
        return;
    }
    m_extractionDialog->showPlanFailure(message);
}

void MainWindow::onExtractionFinished(quint64 requestId, const ExtractionReportSnapshot &report)
{
    if (!m_session.requests().accepts(RequestFamily::Extraction, requestId)) {
        return;
    }
    m_session.endExtraction();
    refreshUiState();
    if (m_extractionDialog != nullptr) {
        m_extractionDialog->showReport(report);
    }
}

void MainWindow::onExtractionFailed(quint64 requestId, const QString &message)
{
    if (!m_session.requests().accepts(RequestFamily::Extraction, requestId)) {
        return;
    }
    m_session.endExtraction();
    refreshUiState();
    if (m_extractionDialog != nullptr) {
        m_extractionDialog->showExtractionFailure(message);
    }
}

void MainWindow::applyHardwareProfile(const HardwareProfileSnapshot &profile)
{
    // Second-line state protection: no profile change may straddle
    // the candidate/commit window or a running extraction; the button
    // is disabled there already, this guards programmatic calls.
    if (m_session.openPhase() != OpenPhase::Idle || m_session.extractionRunning()) {
        return;
    }

    // A new profile invalidates the selection in flight and the
    // overlay on screen; the Files rows, search results and the
    // inspector are not reloaded for a profile change. The candidate
    // suggestions belong to the distribution, not the profile, and
    // stay.
    m_session.requests().invalidate(RequestFamily::Selection);
    m_entries->clearSelectionOverlay();
    m_session.applyProfile(profile);
    updateHardwareButton();
    refreshUiState();
    refreshPersistentStatus();

    // A non-empty profile with a committed distribution is evaluated
    // at once; without one it is simply stored and evaluated as soon
    // as a distribution is committed.
    if (m_session.selectionPhase() == SelectionPhase::Pending) {
        const quint64 requestId = m_session.requests().issue(RequestFamily::Selection);
        emit selectionRequested(requestId, m_session.profile());
    }
}

void MainWindow::onCandidateReady(quint64 productCount,
                                  quint64 diagnosticCount,
                                  const HierarchySnapshot &hierarchy)
{
    QString error;
    if (!m_model->setHierarchy(hierarchy, &error)) {
        // The snapshot violates the model's invariants. Reject the
        // candidate: the worker drops it and keeps the previously
        // committed backend, which still matches the tree on screen.
        m_session.rejectCandidate();
        emit candidateRejected();
        refreshUiState();
        refreshPersistentStatus();
        QMessageBox::critical(this,
                              tr("Failed to Open Distribution"),
                              tr("The backend returned an invalid hierarchy: %1").arg(error));
        return;
    }

    // The tree now shows the candidate; commit it so the backend
    // serves the very object ids the tree carries.
    //
    // The model reset above cleared the browse panes. Between
    // accepting the candidate and the worker's candidateCommitted()
    // the committed backend still holds the old distribution while
    // the tree already carries the new object ids, so tree
    // interaction is locked until the commit lands: no inspector
    // request may be issued in that window.
    //
    // Only now — with the candidate validated and accepted — is the
    // old distribution's search state cleared; a failed or rejected
    // open never touches it.
    //
    // The hardware profile survives, but everything derived from the
    // old distribution dies with it: the selection overlay, the
    // selection request in flight and the candidate suggestions.
    exitSearch();
    m_entries->clearSelectionOverlay();
    m_session.requests().invalidate(RequestFamily::Selection);
    m_session.requests().invalidate(RequestFamily::HardwareCandidates);
    m_session.acceptCandidate({productCount, diagnosticCount});
    refreshUiState();
    refreshPersistentStatus();
    emit candidateAccepted();
}

void MainWindow::onDistributionOpenFailed(const QString &message)
{
    // The backend kept the previously loaded distribution; the GUI
    // must keep showing it too — tree, entries, inspector, hardware
    // profile, selection overlay and suggestions included.
    m_session.openFailed();
    refreshUiState();
    refreshPersistentStatus();
    QMessageBox::critical(this, tr("Failed to Open Distribution"), message);
}

void MainWindow::onCandidateCommitted()
{
    // The backend now serves the object ids the tree carries: the
    // pending distribution becomes the committed one.
    m_session.candidateCommitted();

    // Fresh hardware candidate suggestions for the newly committed
    // distribution.
    const quint64 candidatesId = m_session.requests().issue(RequestFamily::HardwareCandidates);
    emit hardwareCandidatesRequested(candidatesId);

    // A stored profile is evaluated against the new distribution;
    // until the result arrives, no stale overlay is shown.
    if (!m_session.profile().isEmpty()) {
        const quint64 selectionId = m_session.requests().issue(RequestFamily::Selection);
        emit selectionRequested(selectionId, m_session.profile());
    }
    refreshUiState();
    refreshPersistentStatus();
}

void MainWindow::onTreeSelectionChanged(const QModelIndex &current, const QModelIndex &previous)
{
    Q_UNUSED(previous);
    if (!current.isValid()) {
        clearBrowsePanes();
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
    if (!m_session.searchActive() || index != m_treeView->currentIndex()) {
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
    const quint64 detailId = m_session.requests().issue(RequestFamily::Inspector);
    m_inspector->showLoading();
    emit detailRequested(detailId, objectId, kind);

    const quint64 entriesId = m_session.requests().issue(RequestFamily::Entries);
    m_entries->showLoading(
        m_model->data(m_model->index(index.row(), DistributionTreeModel::NameColumn, index.parent()))
            .toString());
    // While the scope loads, the Files view is not an extraction
    // scope.
    refreshUiState();
    emit entriesRequested(entriesId, objectId);
}

void MainWindow::onSearchTextChanged(const QString &text)
{
    if (text.isEmpty()) {
        if (!m_session.searchActive()) {
            return;
        }
        // An empty field means browsing mode, never a "match
        // everything" search: drop the pending search and fall back
        // to the scope the tree still has selected.
        m_session.leaveSearch();
        m_searchDebounce->stop();
        restoreHierarchyScope();
        return;
    }

    // Entering or refining search mode. Every change invalidates both
    // request families, so a late response of an older query can
    // never flash its results; the debounce turns the keystrokes into
    // one request once the user pauses.
    m_session.enterSearch();
    m_session.requests().invalidate(RequestFamily::Entries);
    m_session.requests().invalidate(RequestFamily::Inspector);
    m_inspector->showEmpty();
    m_entries->showLoading(tr("Search: %1").arg(text));
    // While the search loads, the Files view is not an extraction
    // scope.
    refreshUiState();
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
    if (query.isEmpty() || !m_session.searchActive()) {
        return;
    }
    const quint64 requestId = m_session.requests().issue(RequestFamily::Entries);
    emit searchEntriesRequested(requestId, query);
}

void MainWindow::exitSearch()
{
    if (!m_session.searchActive() && m_searchEdit->text().isEmpty()) {
        return;
    }
    // Programmatic clears must not re-enter the search state machine
    // through textChanged.
    const QSignalBlocker blocker(m_searchEdit);
    m_searchEdit->clear();
    m_searchDebounce->stop();
    m_session.leaveSearch();
}

void MainWindow::restoreHierarchyScope()
{
    const QModelIndex current = m_treeView->currentIndex();
    if (current.isValid()) {
        activateHierarchySelection(current);
    } else {
        clearBrowsePanes();
    }
}

void MainWindow::clearBrowsePanes()
{
    // Invalidate the pending requests: a late response must never
    // overwrite the cleared panes.
    m_session.requests().invalidate(RequestFamily::Inspector);
    m_session.requests().invalidate(RequestFamily::Entries);
    m_inspector->showEmpty();
    m_entries->showEmpty();
    refreshUiState();
}

void MainWindow::onEntrySelected(quint64 productId, quint64 entryId)
{
    // An entry pick replaces only the inspector content; the entry
    // list the user is browsing stays untouched.
    const quint64 requestId = m_session.requests().issue(RequestFamily::Inspector);
    m_inspector->showLoading();
    emit entryDetailRequested(requestId, productId, entryId);
}

void MainWindow::onProductDetailReady(quint64 requestId, const ProductDetailSnapshot &detail)
{
    if (!m_session.requests().accepts(RequestFamily::Inspector, requestId)) {
        return;
    }
    m_inspector->showProduct(detail);
    refreshPersistentStatus();
}

void MainWindow::onImageDetailReady(quint64 requestId, const ImageDetailSnapshot &detail)
{
    if (!m_session.requests().accepts(RequestFamily::Inspector, requestId)) {
        return;
    }
    m_inspector->showImage(detail);
    refreshPersistentStatus();
}

void MainWindow::onSubsystemDetailReady(quint64 requestId, const SubsystemDetailSnapshot &detail)
{
    if (!m_session.requests().accepts(RequestFamily::Inspector, requestId)) {
        return;
    }
    m_inspector->showSubsystem(detail);
    refreshPersistentStatus();
}

void MainWindow::onDetailFailed(quint64 requestId, const QString &message)
{
    if (!m_session.requests().accepts(RequestFamily::Inspector, requestId)) {
        // Stale failure: the user has moved on; stay silent.
        return;
    }
    m_inspector->showError(message);
    statusBar()->showMessage(tr("Unable to load details: %1").arg(message));
}

void MainWindow::onEntryDetailReady(quint64 requestId, const EntryDetailSnapshot &detail)
{
    if (!m_session.requests().accepts(RequestFamily::Inspector, requestId)) {
        return;
    }
    m_inspector->showEntry(detail);
    refreshPersistentStatus();
}

void MainWindow::onEntriesReady(quint64 requestId, const EntryListSnapshot &entries)
{
    if (!m_session.requests().accepts(RequestFamily::Entries, requestId)) {
        // A scope or query the user has already left: drop it
        // silently.
        return;
    }
    m_entries->showEntries(entries);
    // A successful render also clears any earlier entries or search
    // error from the status bar, back to the distribution state.
    refreshUiState();
    refreshPersistentStatus();
}

void MainWindow::onEntriesFailed(quint64 requestId, const QString &message)
{
    if (!m_session.requests().accepts(RequestFamily::Entries, requestId)) {
        return;
    }
    m_entries->showError(message);
    refreshUiState();
    statusBar()->showMessage(tr("Unable to load entries: %1").arg(message));
}

void MainWindow::onSelectionReady(quint64 requestId, const SelectionSnapshot &selection)
{
    if (!m_session.requests().accepts(RequestFamily::Selection, requestId)) {
        // A profile the user has already replaced: drop it silently.
        return;
    }
    m_session.selectionSucceeded(static_cast<quint64>(selection.selected.size()),
                                 static_cast<quint64>(selection.conflicts.size()));
    m_entries->setSelectionOverlay(selection);
    refreshUiState();
    refreshPersistentStatus();
}

void MainWindow::onSelectionFailed(quint64 requestId, const QString &message)
{
    if (!m_session.requests().accepts(RequestFamily::Selection, requestId)) {
        return;
    }
    // The profile stays applied and the overlay stays cleared; the
    // error is reflected in the status bar until the state changes.
    m_session.selectionFailed(message);
    refreshUiState();
    refreshPersistentStatus();
}

void MainWindow::onHardwareCandidatesReady(quint64 requestId,
                                           const HardwareCandidatesSnapshot &candidates)
{
    if (!m_session.requests().accepts(RequestFamily::HardwareCandidates, requestId)) {
        return;
    }
    m_session.setCandidates(candidates);
}

void MainWindow::onHardwareCandidatesFailed(quint64 requestId, const QString &message)
{
    Q_UNUSED(message);
    if (!m_session.requests().accepts(RequestFamily::HardwareCandidates, requestId)) {
        return;
    }
    // Suggestions are a convenience, never a gate: the profile
    // editor keeps working with whatever candidates remain (and
    // always allows hand-typed values).
}

void MainWindow::updateHardwareButton()
{
    const HardwareProfileSnapshot &profile = m_session.profile();
    if (profile.isEmpty()) {
        m_hardwareButton->setText(tr("Hardware: Off"));
        m_hardwareButton->setToolTip(
            tr("No hardware profile applied. Every IDB record is shown without a "
               "selection status."));
        return;
    }

    // The summary shows values only (the empty value as a visible
    // "(empty)" placeholder); the tooltip carries the full
    // attribute=value list verbatim, one pair per line.
    QStringList values;
    QStringList lines;
    values.reserve(profile.size());
    lines.reserve(profile.size());
    for (const HardwareValueSnapshot &pair : profile) {
        values.append(pair.value.isEmpty() ? tr("(empty)") : pair.value);
        lines.append(QStringLiteral("%1=%2").arg(pair.attribute, pair.value));
    }
    QString summary;
    if (values.size() <= 4) {
        summary = values.join(QStringLiteral(" / "));
    } else {
        summary = values.mid(0, 3).join(QStringLiteral(" / "))
            + QStringLiteral(" / +%1").arg(values.size() - 3);
    }
    m_hardwareButton->setText(tr("Hardware: %1").arg(summary));
    m_hardwareButton->setToolTip(lines.join(QLatin1Char('\n')));
}
