#include "MainWindow.h"

#include "BackendWorker.h"

#include <QAction>
#include <QFileDialog>
#include <QLabel>
#include <QMenuBar>
#include <QMessageBox>
#include <QPushButton>
#include <QStatusBar>
#include <QThread>
#include <QVBoxLayout>

MainWindow::MainWindow()
{
    setWindowTitle(QStringLiteral("SW Explorer"));
    resize(640, 400);

    m_openAction = new QAction(tr("Open Distribution..."), this);
    connect(m_openAction, &QAction::triggered, this, &MainWindow::chooseDistribution);
    menuBar()->addMenu(tr("&File"))->addAction(m_openAction);

    auto *central = new QWidget(this);
    auto *layout = new QVBoxLayout(central);
    m_openButton = new QPushButton(tr("Open Distribution..."), central);
    m_statusLabel = new QLabel(tr("No distribution loaded."), central);
    m_statusLabel->setAlignment(Qt::AlignCenter);
    layout->addWidget(m_openButton);
    layout->addWidget(m_statusLabel, 1);
    setCentralWidget(central);

    // The button and the menu entry are the same action.
    connect(m_openButton, &QPushButton::clicked, m_openAction, &QAction::trigger);

    statusBar()->showMessage(tr("Ready"));

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
    connect(m_worker,
            &BackendWorker::distributionOpened,
            this,
            &MainWindow::onDistributionOpened);
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
    m_statusLabel->setText(tr("Loading distribution..."));
    statusBar()->showMessage(tr("Loading..."));
    emit openDistributionRequested(path);
}

void MainWindow::onDistributionOpened(quint64 productCount, quint64 diagnosticCount)
{
    QString text = tr("Loaded %1 products").arg(productCount);
    if (diagnosticCount > 0) {
        text += tr(" · %1 diagnostics").arg(diagnosticCount);
    }

    m_hasDistribution = true;
    m_loadedStatusText = text;
    m_statusLabel->setText(text);
    statusBar()->showMessage(text);
    setOpenInProgress(false);
}

void MainWindow::onDistributionOpenFailed(const QString &message)
{
    setOpenInProgress(false);
    // The backend kept the previously loaded distribution; the GUI
    // must keep showing it too.
    m_statusLabel->setText(m_hasDistribution ? m_loadedStatusText
                                             : tr("No distribution loaded."));
    statusBar()->showMessage(tr("Ready"));
    QMessageBox::critical(this, tr("Failed to Open Distribution"), message);
}

void MainWindow::setOpenInProgress(bool inProgress)
{
    m_openAction->setEnabled(!inProgress);
    m_openButton->setEnabled(!inProgress);
}
