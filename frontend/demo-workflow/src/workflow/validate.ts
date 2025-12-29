import type { Edge, Node } from 'reactflow';
import { getNtxNodeType, type ValidationIssue } from '../hooks/useWorkflowEditor';
import { toRecord } from '../utils/toRecord';
import type { ActionsCatalog } from '../types/catalog';
import { computeReachableFromStart } from './graph';

export function validateGraph(nodes: Node[], edges: Edge[], opts?: { catalog?: ActionsCatalog }): ValidationIssue[] {
    const issues: ValidationIssue[] = [];
    const catalog = opts?.catalog;

    const nodeIds = new Set(nodes.map((n) => n.id));
    const startNodes = nodes.filter((n) => getNtxNodeType(n) === 'start');
    const endNodes = nodes.filter((n) => getNtxNodeType(n) === 'end');
    const actionNodes = nodes.filter((n) => getNtxNodeType(n) === 'action');
    const waitNodes = nodes.filter((n) => getNtxNodeType(n) === 'wait');

    if (startNodes.length === 0) {
        issues.push({ level: 'warning', message: 'No start node yet.' });
    } else if (startNodes.length > 1) {
        issues.push({
            level: 'warning',
            message: `Multiple start nodes (${startNodes.length}). Only one will be used for reachability/export.`,
        });
    }

    // Because exporter uses "reachable from start" filtering, unreachable nodes are typically accidental.
    if (startNodes.length >= 1) {
        const reachable = computeReachableFromStart(nodes, edges);
        const unreachable = nodes.filter((n) => !reachable.has(n.id));
        if (unreachable.length > 0) {
            issues.push({
                level: 'warning',
                message: `There are ${unreachable.length} unreachable node(s). Export will only include nodes reachable from start.`,
            });
        }
    }

    if (endNodes.length === 0) {
        issues.push({ level: 'warning', message: 'No end node yet.' });
    }

    if (actionNodes.length === 0) {
        issues.push({ level: 'warning', message: 'No action nodes yet (nothing meaningful to run).' });
    }

    for (const n of actionNodes) {
        const data = toRecord(n.data);
        const actionRef = data.action_ref;
        const call = data.call;
        if (typeof actionRef !== 'string' || actionRef.length === 0) {
            issues.push({ level: 'error', message: `Node ${n.id}: missing action_ref` });
        }
        if (typeof call !== 'string' || call.length === 0) {
            issues.push({ level: 'error', message: `Node ${n.id}: missing call` });
        } else if (catalog) {
            const known = catalog.actions.some((a) => a.summary.id === call);
            if (!known) {
                issues.push({ level: 'warning', message: `Node ${n.id}: call '${call}' not found in actions catalog` });
            }
        }
    }

    for (const n of waitNodes) {
        const data = toRecord(n.data);
        const onObj = toRecord(data.on);
        const event = onObj.event;
        if (typeof event !== 'string' || event.length === 0) {
            issues.push({ level: 'error', message: `Node ${n.id}: wait node missing on.event` });
        }

        // match is optional but in common packet-rx flows it needs action_id.
        const matchObj = toRecord(onObj.match);
        const hasActionId = typeof matchObj.action_id === 'string' && (matchObj.action_id as string).length > 0;
        if (!hasActionId) {
            issues.push({
                level: 'warning',
                message: `Node ${n.id}: wait.on.match.action_id is missing. Export may infer it from incoming action edges; for reliability set it explicitly.`,
            });
        }
    }

    // Start node edge constraints: we typically expect 1 outgoing edge.
    const outgoingCount = new Map<string, number>();
    for (const e of edges) {
        outgoingCount.set(e.source, (outgoingCount.get(e.source) ?? 0) + 1);
    }
    for (const s of startNodes) {
        const out = outgoingCount.get(s.id) ?? 0;
        if (out === 0) {
            issues.push({ level: 'warning', message: `Node ${s.id}: start node has no outgoing edges` });
        } else if (out > 1) {
            issues.push({
                level: 'warning',
                message: `Node ${s.id}: start node has ${out} outgoing edges; exporter will treat the first connection as the primary path.`,
            });
        }
    }
    for (const n of endNodes) {
        const out = outgoingCount.get(n.id) ?? 0;
        if (out > 0) {
            issues.push({ level: 'warning', message: `Node ${n.id}: end node has outgoing edge(s)` });
        }
    }

    for (const e of edges) {
        if (!nodeIds.has(e.source)) {
            issues.push({ level: 'error', message: `Edge ${e.id}: unknown source node ${e.source}` });
        }
        if (!nodeIds.has(e.target)) {
            issues.push({ level: 'error', message: `Edge ${e.id}: unknown target node ${e.target}` });
        }
    }

    return issues;
}

// Export-time blocking checks.
// Rules of thumb:
// - Anything that will definitely make the exported scenario invalid -> error
// - Ambiguity that exporter "guesses" (e.g. wait.action_id inference) -> error at export time
// - Unknown action call when catalog exists -> error at export time
export function validateExportBlocking(nodes: Node[], edges: Edge[], opts?: { catalog?: ActionsCatalog }): ValidationIssue[] {
    const issues: ValidationIssue[] = [];
    const catalog = opts?.catalog;

    const startNodes = nodes.filter((n) => getNtxNodeType(n) === 'start');
    const endNodes = nodes.filter((n) => getNtxNodeType(n) === 'end');
    const actionNodes = nodes.filter((n) => getNtxNodeType(n) === 'action');
    const waitNodes = nodes.filter((n) => getNtxNodeType(n) === 'wait');

    if (startNodes.length !== 1) {
        issues.push({
            level: 'error',
            message: startNodes.length === 0 ? 'Export blocked: missing start node.' : 'Export blocked: multiple start nodes (must be exactly 1).',
        });
    }
    if (endNodes.length < 1) {
        issues.push({ level: 'error', message: 'Export blocked: missing end node.' });
    }
    if (actionNodes.length < 1) {
        issues.push({ level: 'error', message: 'Export blocked: missing action node (nothing to run).' });
    }

    // End nodes must have no outgoing edges.
    const outgoingCount = new Map<string, number>();
    for (const e of edges) outgoingCount.set(e.source, (outgoingCount.get(e.source) ?? 0) + 1);
    for (const n of endNodes) {
        const out = outgoingCount.get(n.id) ?? 0;
        if (out > 0) {
            issues.push({ level: 'error', nodeId: n.id, message: `Export blocked: end node ${n.id} has outgoing edge(s).` });
        }
    }

    // Reachability: exporter drops unreachable nodes; at export time that’s almost always a mistake.
    if (startNodes.length >= 1) {
        const reachable = computeReachableFromStart(nodes, edges);
        const unreachable = nodes.filter((n) => !reachable.has(n.id));
        if (unreachable.length > 0) {
            issues.push({
                level: 'error',
                message: `Export blocked: ${unreachable.length} unreachable node(s). Connect them from start or delete them.`,
            });
        }
    }

    // Action nodes must have action_ref + call; and call must exist in catalog when catalog is present.
    for (const n of actionNodes) {
        const data = toRecord(n.data);
        const actionRef = data.action_ref;
        const call = data.call;
        if (typeof actionRef !== 'string' || actionRef.length === 0) {
            issues.push({ level: 'error', nodeId: n.id, message: `Export blocked: node ${n.id} missing action_ref.` });
        }
        if (typeof call !== 'string' || call.length === 0) {
            issues.push({ level: 'error', nodeId: n.id, message: `Export blocked: node ${n.id} missing call.` });
        } else if (catalog) {
            const known = catalog.actions.some((a) => a.summary.id === call);
            if (!known) {
                issues.push({
                    level: 'error',
                    nodeId: n.id,
                    message: `Export blocked: node ${n.id} call '${call}' not found in actions catalog.`,
                });
            }
        }
    }

    // Wait nodes: require on.event; and require match.action_id either explicitly or inferable from incoming action nodes.
    const incomingByTarget = new Map<string, string[]>();
    for (const e of edges) {
        const arr = incomingByTarget.get(e.target) ?? [];
        arr.push(e.source);
        incomingByTarget.set(e.target, arr);
    }

    for (const n of waitNodes) {
        const data = toRecord(n.data);
        const onObj = toRecord(data.on);
        const event = onObj.event;
        if (typeof event !== 'string' || event.length === 0) {
            issues.push({ level: 'error', nodeId: n.id, message: `Export blocked: wait node ${n.id} missing on.event.` });
        }

        const matchObj = toRecord(onObj.match);
        const explicitActionId = typeof matchObj.action_id === 'string' && (matchObj.action_id as string).length > 0 ? (matchObj.action_id as string) : null;
        if (explicitActionId) continue;

        const incoming = incomingByTarget.get(n.id) ?? [];
        const inferable = incoming
            .map((id) => nodes.find((x) => x.id === id) ?? null)
            .some((x) => {
                if (!x) return false;
                if (getNtxNodeType(x) !== 'action') return false;
                const upstream = toRecord(x.data);
                return typeof upstream.action_ref === 'string' && (upstream.action_ref as string).length > 0;
            });

        if (!inferable) {
            issues.push({
                level: 'error',
                nodeId: n.id,
                message: `Export blocked: wait node ${n.id} missing match.action_id and no incoming action edge to infer from.`,
            });
        }
    }

    return issues;
}
