#pragma once

#include "file_table_model.h"

#include <QFutureWatcher>
#include <QMainWindow>
#include <QPoint>

class QComboBox;
class QLineEdit;
class QTableView;
class QAction;
class QModelIndex;
class QTimer;

class MainWindow : public QMainWindow {
    Q_OBJECT
public:
    explicit MainWindow(QWidget *parent = nullptr);

private slots:
    void openDistDirectory();
    void openIdbFile();
    void refreshProducts();
    void scanCurrentProduct();
    void updateFilters();
    void activateRow(const QModelIndex &index);
    void showTableContextMenu(const QPoint &pos);
    void goUpDirectory();
    void extractSelected();
    void extractAll();
    void requestStop();
    void showAboutDialog();

private:
    struct ScanTaskResult {
        swcore::ParseResult parsed;
        QString error;
        QString distDir;
        QString product;
    };

    void buildUi();
    void buildMenus();
    void buildToolBar();
    void setDistDirectory(const QString &path);
    QVector<swcore::FileEntry> selectedEntries() const;
    QString selectedRowPathsText() const;
    void runExtraction(const QVector<swcore::FileEntry> &entries);
    void updatePathDisplay();
    void refreshStatus();

    QString m_distDirPath;
    QString m_lastOutDirPath;
    bool m_stopRequested = false;
    bool m_scanQueued = false;

    QComboBox *m_productCombo = nullptr;
    QLineEdit *m_maskEdit = nullptr;
    QLineEdit *m_searchEdit = nullptr;
    QLineEdit *m_pathEdit = nullptr;
    QTableView *m_tableView = nullptr;
    FileTableModel *m_tableModel = nullptr;

    QAction *m_openDistAction = nullptr;
    QAction *m_openIdbAction = nullptr;
    QAction *m_scanAction = nullptr;
    QAction *m_upAction = nullptr;
    QAction *m_extractSelectedAction = nullptr;
    QAction *m_extractAllAction = nullptr;
    QAction *m_stopAction = nullptr;
    QAction *m_refreshAction = nullptr;
    QAction *m_noDecompressAction = nullptr;
    QAction *m_keepZAction = nullptr;
    QAction *m_continueOnErrorAction = nullptr;
    QTimer *m_filterTimer = nullptr;
    QFutureWatcher<ScanTaskResult> *m_scanWatcher = nullptr;
};
