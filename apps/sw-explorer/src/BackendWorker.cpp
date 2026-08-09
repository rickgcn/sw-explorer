#include "rust/cxx.h"

#include "BackendWorker.h"

namespace {

HierarchyKind toQtKind(sw::ObjectKind kind)
{
    switch (kind) {
    case sw::ObjectKind::Product:
        return HierarchyKind::Product;
    case sw::ObjectKind::Image:
        return HierarchyKind::Image;
    case sw::ObjectKind::Subsystem:
        return HierarchyKind::Subsystem;
    }
    Q_UNREACHABLE();
}

QString toQString(const rust::String &value)
{
    return QString::fromUtf8(value.data(), static_cast<qsizetype>(value.size()));
}

} // namespace

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

    // Load into a throwaway candidate: the committed backend stays
    // untouched until the GUI has accepted the candidate's hierarchy.
    rust::Box<sw::Backend> candidate = sw::new_backend();
    try {
        // Open first, then fetch the hierarchy of the candidate: the
        // GUI receives both in a single signal and never has to ask
        // twice.
        const sw::DistributionSummary summary = candidate->open_distribution(rustPath);
        const rust::Vec<sw::HierarchyNode> nodes = candidate->hierarchy();

        // rust:: types stay on this side of the boundary; the snapshot
        // is plain Qt data.
        HierarchySnapshot snapshot;
        snapshot.reserve(static_cast<qsizetype>(nodes.size()));
        for (const sw::HierarchyNode &node : nodes) {
            HierarchyNodeSnapshot item;
            item.id = node.id;
            item.parentId = node.parent_id;
            item.kind = toQtKind(node.kind);
            item.name = toQString(node.name);
            item.title = toQString(node.title);
            item.entryCount = node.entry_count;
            item.descriptorPresent = node.descriptor_present;
            item.idbPresent = node.idb_present;
            snapshot.append(item);
        }

        m_candidate = std::move(candidate);
        emit candidateReady(summary.product_count, summary.diagnostic_count, snapshot);
    } catch (const rust::Error &error) {
        // The candidate is destroyed with this frame; the committed
        // backend was never involved.
        emit distributionOpenFailed(QString::fromUtf8(error.what()));
    }
}

void BackendWorker::commitCandidate()
{
    if (m_candidate.has_value()) {
        m_backend = std::move(*m_candidate);
        m_candidate.reset();
    }
}

void BackendWorker::discardCandidate()
{
    m_candidate.reset();
}
