#pragma once

#include <QMainWindow>

QT_BEGIN_NAMESPACE
class QAction;
class QLabel;
class QPushButton;
class QThread;
QT_END_NAMESPACE

class BackendWorker;

// Bootstrap main window: a single "Open Distribution..." action whose
// work runs on the BackendWorker thread.
class MainWindow : public QMainWindow
{
    Q_OBJECT

public:
    MainWindow();
    ~MainWindow() override;

signals:
    void openDistributionRequested(const QString &path);

private slots:
    void chooseDistribution();
    void onDistributionOpened(quint64 productCount, quint64 diagnosticCount);
    void onDistributionOpenFailed(const QString &message);

private:
    void setOpenInProgress(bool inProgress);

    QAction *m_openAction = nullptr;
    QPushButton *m_openButton = nullptr;
    QLabel *m_statusLabel = nullptr;

    // Last successfully loaded state, kept in sync with the backend:
    // a failed open must leave both sides showing the old distribution.
    bool m_hasDistribution = false;
    QString m_loadedStatusText;

    QThread *m_workerThread = nullptr;
    BackendWorker *m_worker = nullptr;
};
