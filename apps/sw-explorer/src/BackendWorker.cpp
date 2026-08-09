#include "rust/cxx.h"

#include "BackendWorker.h"

BackendWorker::BackendWorker(QObject *parent)
    : QObject(parent)
    , m_backend(sw::new_backend())
{
}

void BackendWorker::openDistribution(const QString &path)
{
    // QString -> rust::Str goes through an explicit UTF-8 byte array;
    // the current locale is never involved.
    const QByteArray utf8 = path.toUtf8();
    const rust::Str rustPath(utf8.constData(), static_cast<std::size_t>(utf8.size()));

    try {
        const sw::DistributionSummary summary = m_backend->open_distribution(rustPath);
        emit distributionOpened(summary.product_count, summary.diagnostic_count);
    } catch (const rust::Error &error) {
        emit distributionOpenFailed(QString::fromUtf8(error.what()));
    }
}
