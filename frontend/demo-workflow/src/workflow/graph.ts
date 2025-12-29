import type { Edge, Node } from 'reactflow';
import { getNtxNodeType } from '../hooks/useWorkflowEditor';

export function computeReachableFromStart(nodes: Node[], edges: Edge[]): Set<string> {
    const start = nodes.find((n) => getNtxNodeType(n) === 'start');
    if (!start) return new Set();

    const outgoing = new Map<string, string[]>();
    for (const e of edges) {
        const arr = outgoing.get(e.source) ?? [];
        arr.push(e.target);
        outgoing.set(e.source, arr);
    }

    const visited = new Set<string>();
    const stack = [start.id];
    while (stack.length) {
        const id = stack.pop()!;
        if (visited.has(id)) continue;
        visited.add(id);
        for (const to of outgoing.get(id) ?? []) {
            if (!visited.has(to)) stack.push(to);
        }
    }

    return visited;
}
