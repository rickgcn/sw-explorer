#include "MainWindow.h"

#include "BackendWorker.h"
#include "models/DistributionTreeModel.h"

#include <QAction>
#include <QFileDialog>
#include <QHeaderView>
#include <QItemSelectionModel>
#include <QLabel>
#include <QMenuBar>
#include <QMessageBox>
#include <QSplitter>
#include <QStatusBar>
#include <QThread>
#include <QTreeView>
#include <QVBoxLayout>

MainWindow::MainWindow()
{
    setWindowTitle(QStringLiteral("SW Explorer"));
    resize(960, 600);

    m_openAction = new QAction(tr("Open Distribution..."), this);
    connect(m_openAction, &QAction::triggered, this, &MainWindow::chooseDistribution);
    menuBar()->addMenu(tr("&File"))->addAction(m_openAction);

    // Left: the distribution hierarchy. Right: the future content
    // area, for now a selection placeholder.
    m_model = new DistributionTreeModel(this);
    m_treeView = new QTreeView;
    m_treeView->setModel(m_model);
    m_treeView->setUniformRowHeights(true);
    m_treeView->header()->setSectionResizeMode(DistributionTreeModel::NameColumn,
                                               QHeaderView::Stretch);
    m_treeView->header()->setSectionResizeMode(DistributionTreeModel::EntriesColumn,
                                               QHeaderView::ResizeToContents);

    auto *rightPanel = new QWidget;
    auto *rightLayout = new QVBoxLayout(rightPanel);
    m_identityLabel = new QLabel(tr("Select an item"), rightPanel);
    QFont identityFont = m_identityLabel->font();
    identityFont.setBold(true);
    m_identityLabel->setFont(identityFont);
    m_kindLabel = new QLabel(rightPanel);
    rightLayout->addStretch();
    rightLayout->addWidget(m_identityLabel, 0, Qt::AlignCenter);
    rightLayout->addWidget(m_kindLabel, 0, Qt::AlignCenter);
    rightLayout->addStretch();

    auto *splitter = new QSplitter(this);
    splitter->addWidget(m_treeView);
    splitter->addWidget(rightPanel);
    splitter->setStretchFactor(0, 0);
    splitter->setStretchFactor(1, 1);
    splitter->setSizes({320, 640});
    setCentralWidget(splitter);

    connect(m_treeView->selectionModel(),
            &QItemSelectionModel::currentChanged,
            this,
            &MainWindow::onTreeSelectionChanged);
    connect(m_model, &QAbstractItemModel::modelReset, this, &MainWindow::clearSelection);

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
    connect(m_worker, &BackendWorker::candidateReady, this, &MainWindow::onCandidateReady);
    connect(m_worker,
            &BackendWorker::distributionOpenFailed,
            this,
            &MainWindow::onDistributionOpenFailed);
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

void MainWindow::onTreeSelectionChanged(const QModelIndex &current, const QModelIndex &previous)
{
    Q_UNUSED(previous);
    if (!current.isValid()) {
        clearSelection();
        return;
    }

    // QModelIndex -> HierarchyItem -> object identity: proves the
    // selection chain from view to backend object id and kind.
    m_identityLabel->setText(m_model->identity(current));
    switch (m_model->kind(current)) {
    case HierarchyKind::Product:
        m_kindLabel->setText(tr("Product"));
        break;
    case HierarchyKind::Image:
        m_kindLabel->setText(tr("Image"));
        break;
    case HierarchyKind::Subsystem:
        m_kindLabel->setText(tr("Subsystem"));
        break;
    }
}

void MainWindow::clearSelection()
{
    m_identityLabel->setText(tr("Select an item"));
    m_kindLabel->clear();
}

void MainWindow::setOpenInProgress(bool inProgress)
{
    m_openAction->setEnabled(!inProgress);
}
