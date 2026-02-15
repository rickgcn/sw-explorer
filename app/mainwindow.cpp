#include "mainwindow.h"

#include "swcore/extractor.h"
#include "swcore/idb_parser.h"

#include <QAction>
#include <QApplication>
#include <QClipboard>
#include <QComboBox>
#include <QDir>
#include <QElapsedTimer>
#include <QFileDialog>
#include <QFileInfo>
#include <QHeaderView>
#include <QHBoxLayout>
#include <QItemSelectionModel>
#include <QKeySequence>
#include <QLabel>
#include <QLineEdit>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QModelIndex>
#include <QProgressDialog>
#include <QStringList>
#include <QSize>
#include <QStatusBar>
#include <QStyle>
#include <QTableView>
#include <QToolBar>
#include <QToolButton>
#include <QTimer>
#include <QVBoxLayout>
#include <QWidget>
#include <QtConcurrent/QtConcurrentRun>

#include <algorithm>

namespace {

QIcon toolbarIcon(QWidget *w, const QString &resourcePath, QStyle::StandardPixmap fallback) {
    QIcon icon(resourcePath);
    if (!icon.isNull()) {
        return icon;
    }
    return w->style()->standardIcon(fallback);
}

} // namespace

MainWindow::MainWindow(QWidget *parent) : QMainWindow(parent) {
    setWindowTitle("sw-explorer");
    resize(1200, 760);
    m_filterTimer = new QTimer(this);
    m_filterTimer->setSingleShot(true);
    m_filterTimer->setInterval(180);
    connect(m_filterTimer, &QTimer::timeout, this, &MainWindow::updateFilters);

    m_scanWatcher = new QFutureWatcher<ScanTaskResult>(this);
    connect(m_scanWatcher, &QFutureWatcher<ScanTaskResult>::finished, this, [this]() {
        m_scanAction->setEnabled(true);
        const ScanTaskResult result = m_scanWatcher->result();
        const bool sameTarget = (result.distDir == m_distDirPath && result.product == m_productCombo->currentText());
        if (sameTarget) {
            if (!result.error.isEmpty()) {
                QMessageBox::critical(this, "Scan Error", result.error);
            } else {
                m_tableModel->setEntries(result.parsed.entries);
                updateFilters();
                if (!result.parsed.warnings.isEmpty()) {
                    statusBar()->showMessage(QString("Loaded with %1 warnings").arg(result.parsed.warnings.size()), 5000);
                } else {
                    statusBar()->showMessage(QString("Loaded %1 entries").arg(result.parsed.entries.size()), 3000);
                }
            }
        }

        if (m_scanQueued) {
            m_scanQueued = false;
            scanCurrentProduct();
        }
    });

    buildMenus();
    buildUi();
    buildToolBar();
    refreshStatus();
}

void MainWindow::buildUi() {
    QWidget *central = new QWidget(this);
    auto *rootLayout = new QVBoxLayout(central);
    rootLayout->setContentsMargins(8, 8, 8, 8);
    rootLayout->setSpacing(6);

    m_productCombo = new QComboBox(this);
    m_productCombo->setMinimumWidth(180);
    m_maskEdit = new QLineEdit("*", this);
    m_maskEdit->setMinimumWidth(160);
    m_searchEdit = new QLineEdit(this);
    m_searchEdit->setMinimumWidth(180);
    m_searchEdit->setPlaceholderText("Name contains...");

    auto *pathLayout = new QHBoxLayout();
    auto *upPathButton = new QToolButton(central);
    upPathButton->setDefaultAction(m_upAction);
    upPathButton->setAutoRaise(true);
    upPathButton->setToolButtonStyle(Qt::ToolButtonIconOnly);
    pathLayout->addWidget(upPathButton);
    m_pathEdit = new QLineEdit(central);
    m_pathEdit->setReadOnly(true);
    pathLayout->addWidget(m_pathEdit, 1);
    rootLayout->addLayout(pathLayout);

    m_tableModel = new FileTableModel(this);

    m_tableView = new QTableView(central);
    m_tableView->setModel(m_tableModel);
    m_tableView->setSelectionBehavior(QAbstractItemView::SelectRows);
    m_tableView->setSelectionMode(QAbstractItemView::ExtendedSelection);
    m_tableView->setAlternatingRowColors(true);
    m_tableView->setSortingEnabled(false);
    m_tableView->setContextMenuPolicy(Qt::CustomContextMenu);
    m_tableView->verticalHeader()->setVisible(false);
    auto *header = m_tableView->horizontalHeader();
    header->setStretchLastSection(false);
    header->setSectionResizeMode(QHeaderView::Interactive);
    header->setMinimumSectionSize(48);
    m_tableView->setColumnWidth(0, 420);
    m_tableView->setColumnWidth(1, 90);
    m_tableView->setColumnWidth(2, 90);
    m_tableView->setColumnWidth(3, 70);
    m_tableView->setColumnWidth(4, 220);
    m_tableView->setColumnWidth(5, 160);
    m_tableView->setColumnWidth(6, 100);
    rootLayout->addWidget(m_tableView, 1);

    setCentralWidget(central);

    connect(m_productCombo, &QComboBox::currentTextChanged, this, &MainWindow::scanCurrentProduct);
    connect(m_maskEdit, &QLineEdit::textChanged, this, [this]() { m_filterTimer->start(); });
    connect(m_searchEdit, &QLineEdit::textChanged, this, [this]() { m_filterTimer->start(); });
    connect(m_tableView->selectionModel(), &QItemSelectionModel::selectionChanged, this, &MainWindow::refreshStatus);
    connect(m_tableView, &QTableView::doubleClicked, this, &MainWindow::activateRow);
    connect(m_tableView, &QTableView::customContextMenuRequested, this, &MainWindow::showTableContextMenu);

    updatePathDisplay();
}

void MainWindow::buildMenus() {
    m_openDistAction = new QAction("Open Dist...", this);
    m_openDistAction->setShortcut(QKeySequence::Open);
    m_openDistAction->setIcon(toolbarIcon(this, ":/icons/open-folder.svg", QStyle::SP_DialogOpenButton));
    connect(m_openDistAction, &QAction::triggered, this, &MainWindow::openDistDirectory);

    m_openIdbAction = new QAction("Open IDB...", this);
    m_openIdbAction->setShortcut(QKeySequence(Qt::CTRL | Qt::SHIFT | Qt::Key_O));
    m_openIdbAction->setIcon(toolbarIcon(this, ":/icons/open-file.svg", QStyle::SP_FileIcon));
    connect(m_openIdbAction, &QAction::triggered, this, &MainWindow::openIdbFile);

    m_scanAction = new QAction("Scan", this);
    m_scanAction->setIcon(toolbarIcon(this, ":/icons/scan.svg", QStyle::SP_BrowserReload));
    connect(m_scanAction, &QAction::triggered, this, &MainWindow::scanCurrentProduct);

    m_upAction = new QAction("Up", this);
    m_upAction->setShortcut(QKeySequence(Qt::ALT | Qt::Key_Up));
    m_upAction->setIcon(toolbarIcon(this, ":/icons/up.svg", QStyle::SP_ArrowUp));
    connect(m_upAction, &QAction::triggered, this, &MainWindow::goUpDirectory);

    m_extractSelectedAction = new QAction("Extract Selected...", this);
    m_extractSelectedAction->setIcon(toolbarIcon(this, ":/icons/extract-selected.svg", QStyle::SP_DialogSaveButton));
    connect(m_extractSelectedAction, &QAction::triggered, this, &MainWindow::extractSelected);

    m_extractAllAction = new QAction("Extract Here Tree...", this);
    m_extractAllAction->setIcon(toolbarIcon(this, ":/icons/extract-tree.svg", QStyle::SP_DialogApplyButton));
    connect(m_extractAllAction, &QAction::triggered, this, &MainWindow::extractAll);

    m_stopAction = new QAction("Stop", this);
    m_stopAction->setEnabled(false);
    m_stopAction->setIcon(toolbarIcon(this, ":/icons/stop.svg", QStyle::SP_BrowserStop));
    connect(m_stopAction, &QAction::triggered, this, &MainWindow::requestStop);

    m_refreshAction = new QAction("Refresh", this);
    m_refreshAction->setIcon(toolbarIcon(this, ":/icons/refresh.svg", QStyle::SP_BrowserReload));
    connect(m_refreshAction, &QAction::triggered, this, &MainWindow::refreshProducts);

    m_noDecompressAction = new QAction("No Decompress (.Z only)", this);
    m_noDecompressAction->setCheckable(true);

    m_keepZAction = new QAction("Keep .Z files", this);
    m_keepZAction->setCheckable(true);

    m_continueOnErrorAction = new QAction("Continue on error", this);
    m_continueOnErrorAction->setCheckable(true);
    m_continueOnErrorAction->setChecked(true);

    QMenu *fileMenu = menuBar()->addMenu("File");
    fileMenu->addAction(m_openDistAction);
    fileMenu->addAction(m_openIdbAction);
    fileMenu->addAction(m_scanAction);
    fileMenu->addAction(m_upAction);
    fileMenu->addSeparator();
    fileMenu->addAction(m_extractSelectedAction);
    fileMenu->addAction(m_extractAllAction);
    fileMenu->addAction(m_stopAction);
    fileMenu->addSeparator();
    fileMenu->addAction("Exit", this, &QWidget::close);

    QMenu *viewMenu = menuBar()->addMenu("View");
    viewMenu->addAction(m_upAction);
    viewMenu->addAction(m_refreshAction);

    QMenu *toolsMenu = menuBar()->addMenu("Tools");
    toolsMenu->addAction(m_noDecompressAction);
    toolsMenu->addAction(m_keepZAction);
    toolsMenu->addAction(m_continueOnErrorAction);

    QMenu *helpMenu = menuBar()->addMenu("Help");
    helpMenu->addAction("About", this, &MainWindow::showAboutDialog);
}

void MainWindow::buildToolBar() {
    QToolBar *tb = addToolBar("Main");
    tb->setMovable(false);
    tb->setToolButtonStyle(Qt::ToolButtonIconOnly);
    tb->setIconSize(QSize(18, 18));
    tb->addAction(m_openDistAction);
    tb->addAction(m_openIdbAction);
    tb->addAction(m_scanAction);
    tb->addSeparator();
    tb->addAction(m_extractSelectedAction);
    tb->addAction(m_extractAllAction);
    tb->addAction(m_stopAction);
    tb->addSeparator();
    tb->addAction(m_refreshAction);
    tb->addSeparator();
    tb->addWidget(new QLabel("Product:", tb));
    tb->addWidget(m_productCombo);
    tb->addWidget(new QLabel("Mask:", tb));
    tb->addWidget(m_maskEdit);
    tb->addWidget(new QLabel("Filter:", tb));
    tb->addWidget(m_searchEdit);
}

void MainWindow::openDistDirectory() {
    const QString start = m_distDirPath.isEmpty() ? QDir::currentPath() : m_distDirPath;
    const QString dir = QFileDialog::getExistingDirectory(this, "Open dist directory", start);
    if (dir.isEmpty()) {
        return;
    }
    setDistDirectory(dir);
}

void MainWindow::openIdbFile() {
    const QString start = m_distDirPath.isEmpty() ? QDir::currentPath() : m_distDirPath;
    const QString idbPath = QFileDialog::getOpenFileName(this, "Open idb file", start, "IDB files (*.idb)");
    if (idbPath.isEmpty()) {
        return;
    }

    const QFileInfo fi(idbPath);
    setDistDirectory(fi.absolutePath());

    const QString product = fi.completeBaseName();
    const int idx = m_productCombo->findText(product);
    if (idx >= 0) {
        m_productCombo->setCurrentIndex(idx);
        scanCurrentProduct();
    } else {
        QMessageBox::warning(this,
                             "Open IDB",
                             QString("Cannot find product '%1' in current dist list.").arg(product));
    }
}

void MainWindow::setDistDirectory(const QString &path) {
    m_distDirPath = path;
    setWindowTitle(QString("sw-explorer - %1").arg(m_distDirPath));
    refreshProducts();
}

void MainWindow::refreshProducts() {
    const QString current = m_productCombo->currentText();
    m_productCombo->blockSignals(true);
    m_productCombo->clear();

    if (!m_distDirPath.isEmpty()) {
        const QStringList products = swcore::IdbParser::findProducts(m_distDirPath);
        m_productCombo->addItems(products);
    }

    int restore = m_productCombo->findText(current);
    if (restore < 0) {
        restore = 0;
    }
    if (m_productCombo->count() > 0) {
        m_productCombo->setCurrentIndex(restore);
    }
    m_productCombo->blockSignals(false);

    scanCurrentProduct();
}

void MainWindow::scanCurrentProduct() {
    if (m_distDirPath.isEmpty() || m_productCombo->currentText().isEmpty()) {
        if (m_scanWatcher->isRunning()) {
            m_scanQueued = false;
        }
        m_tableModel->setEntries({});
        updatePathDisplay();
        refreshStatus();
        return;
    }

    if (m_scanWatcher->isRunning()) {
        m_scanQueued = true;
        return;
    }

    const QString distDir = m_distDirPath;
    const QString product = m_productCombo->currentText();
    m_scanAction->setEnabled(false);
    statusBar()->showMessage(QString("Scanning %1...").arg(product));

    auto future = QtConcurrent::run([distDir, product]() {
        ScanTaskResult result;
        result.distDir = distDir;
        result.product = product;
        result.parsed = swcore::IdbParser::parse(distDir, product, &result.error);
        return result;
    });
    m_scanWatcher->setFuture(future);
}

void MainWindow::updateFilters() {
    m_tableModel->setFilters(m_maskEdit->text(), m_searchEdit->text());
    updatePathDisplay();
    refreshStatus();
}

void MainWindow::activateRow(const QModelIndex &index) {
    if (!index.isValid()) {
        return;
    }
    const int row = index.row();
    const FileTableModel::RowKind kind = m_tableModel->rowKind(row);
    if (kind == FileTableModel::RowKind::Parent) {
        goUpDirectory();
        return;
    }
    if (kind == FileTableModel::RowKind::Directory || kind == FileTableModel::RowKind::DirectoryLink) {
        m_tableModel->setCurrentDirectory(m_tableModel->rowPath(row));
        updatePathDisplay();
        refreshStatus();
    }
}

void MainWindow::goUpDirectory() {
    if (!m_tableModel->canGoUp()) {
        return;
    }
    m_tableModel->goUp();
    updatePathDisplay();
    refreshStatus();
}

QVector<swcore::FileEntry> MainWindow::selectedEntries() const {
    const QModelIndexList rows = m_tableView->selectionModel()->selectedRows();
    return m_tableModel->entriesForRows(rows);
}

QString MainWindow::selectedRowPathsText() const {
    if (!m_tableView || !m_tableView->selectionModel()) {
        return {};
    }
    const QModelIndexList rows = m_tableView->selectionModel()->selectedRows();
    QStringList paths;
    paths.reserve(rows.size());
    for (const QModelIndex &idx : rows) {
        const QString p = m_tableModel->rowSourcePath(idx.row());
        if (p.isEmpty()) {
            continue;
        }
        paths.push_back("/" + p);
    }
    paths.removeDuplicates();
    return paths.join('\n');
}

void MainWindow::showTableContextMenu(const QPoint &pos) {
    QModelIndex index = m_tableView->indexAt(pos);
    if (index.isValid() && m_tableView->selectionModel()) {
        if (!m_tableView->selectionModel()->isSelected(index)) {
            m_tableView->selectRow(index.row());
        }
    }

    QMenu menu(this);

    if (index.isValid()) {
        const FileTableModel::RowKind kind = m_tableModel->rowKind(index.row());
        if (kind == FileTableModel::RowKind::Parent) {
            QAction *up = menu.addAction(m_upAction->icon(), "Up");
            connect(up, &QAction::triggered, this, &MainWindow::goUpDirectory);
            menu.addSeparator();
        } else if (kind == FileTableModel::RowKind::Directory || kind == FileTableModel::RowKind::DirectoryLink) {
            QAction *open = menu.addAction("Open");
            connect(open, &QAction::triggered, this, [this, index]() { activateRow(index); });
            menu.addSeparator();
        }
    }

    menu.addAction(m_extractSelectedAction);
    menu.addAction(m_extractAllAction);

    const QString pathsText = selectedRowPathsText();
    if (!pathsText.isEmpty()) {
        menu.addSeparator();
        QAction *copyPaths = menu.addAction("Copy Path");
        connect(copyPaths, &QAction::triggered, this, [pathsText]() {
            if (QClipboard *cb = qApp->clipboard()) {
                cb->setText(pathsText);
            }
        });
    }

    menu.exec(m_tableView->viewport()->mapToGlobal(pos));
}

void MainWindow::extractSelected() {
    const QVector<swcore::FileEntry> entries = selectedEntries();
    if (entries.isEmpty()) {
        QMessageBox::information(this, "Extract", "No file or directory selected.");
        return;
    }
    runExtraction(entries);
}

void MainWindow::extractAll() {
    runExtraction(m_tableModel->entriesInCurrentTree());
}

void MainWindow::runExtraction(const QVector<swcore::FileEntry> &entries) {
    if (entries.isEmpty()) {
        QMessageBox::information(this, "Extract", "No entries available.");
        return;
    }
    if (m_distDirPath.isEmpty()) {
        QMessageBox::warning(this, "Extract", "Please open a dist directory first.");
        return;
    }

    const QString start = m_lastOutDirPath.isEmpty() ? m_distDirPath : m_lastOutDirPath;
    const QString outDir = QFileDialog::getExistingDirectory(this, "Extract to directory", start);
    if (outDir.isEmpty()) {
        return;
    }
    m_lastOutDirPath = outDir;

    swcore::ExtractOptions options;
    options.noDecompress = m_noDecompressAction->isChecked();
    options.keepZ = m_keepZAction->isChecked();
    options.continueOnError = m_continueOnErrorAction->isChecked();

    QProgressDialog progress("Extracting...", "Stop", 0, entries.size(), this);
    progress.setWindowModality(Qt::ApplicationModal);
    progress.setMinimumDuration(0);
    progress.setValue(0);

    m_stopRequested = false;
    m_stopAction->setEnabled(true);
    QElapsedTimer tick;
    tick.start();
    qint64 lastUiUpdateMs = -1;

    const swcore::ExtractResult result =
        swcore::DistExtractor::extract(m_distDirPath, entries, outDir, options,
                                       [&](int current, int total, const QString &name) {
                                           if (progress.wasCanceled() || m_stopRequested) {
                                               return false;
                                           }

                                           const qint64 now = tick.elapsed();
                                           const bool shouldRefreshUi = (current == 1 || current == total || lastUiUpdateMs < 0 ||
                                                                         (now - lastUiUpdateMs) >= 50);
                                           if (shouldRefreshUi) {
                                               lastUiUpdateMs = now;
                                               progress.setMaximum(total);
                                               progress.setValue(std::max(0, current - 1));
                                               progress.setLabelText(
                                                   QString("Extracting %1 (%2/%3)").arg(name).arg(current).arg(total));
                                               qApp->processEvents();
                                               if (progress.wasCanceled() || m_stopRequested) {
                                                   return false;
                                               }
                                           }
                                           return true;
                                       });

    m_stopAction->setEnabled(false);
    progress.setValue(entries.size());

    QString summary = QString("Total: %1\nExtracted: %2\nSkipped: %3\nErrors: %4")
                          .arg(result.total)
                          .arg(result.extracted)
                          .arg(result.skipped)
                          .arg(result.errors);
    if (result.canceled) {
        summary += "\nCanceled: yes";
    }

    if (!result.errorMessages.isEmpty()) {
        summary += "\n\nFirst errors:\n";
        const int n = std::min(5, int(result.errorMessages.size()));
        for (int i = 0; i < n; ++i) {
            summary += result.errorMessages.at(i) + "\n";
        }
    }

    if (result.errors > 0) {
        QMessageBox::warning(this, "Extract finished", summary);
    } else {
        QMessageBox::information(this, "Extract finished", summary);
    }
    refreshStatus();
}

void MainWindow::requestStop() {
    m_stopRequested = true;
}

void MainWindow::showAboutDialog() {
    QMessageBox::about(this,
                       "About",
                       "sw-explorer\n"
                       "IRIX dist browser/extractor");
}

void MainWindow::updatePathDisplay() {
    const QString dir = m_tableModel ? m_tableModel->currentDirectory() : QString();
    m_pathEdit->setText(dir.isEmpty() ? "/" : "/" + dir);
    if (m_upAction) {
        m_upAction->setEnabled(m_tableModel && m_tableModel->canGoUp());
    }
}

void MainWindow::refreshStatus() {
    const int total = m_tableModel ? m_tableModel->totalFilteredEntryCount() : 0;
    const int visibleItems = m_tableModel ? m_tableModel->rowCount() : 0;
    int selected = 0;
    if (m_tableView && m_tableView->selectionModel()) {
        selected = m_tableView->selectionModel()->selectedRows().size();
    }
    statusBar()->showMessage(QString("Filtered entries: %1  Items in dir: %2  Selected rows: %3")
                                 .arg(total)
                                 .arg(visibleItems)
                                 .arg(selected));
}
