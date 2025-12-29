import { buildScenarioYaml } from './export';
import { validateExportBlocking, validateGraph } from './validate';

import { parse as parseYaml } from 'yaml';

// reactflow types are erased at runtime; we just need compatible shapes.
import type { Edge, Node } from 'reactflow';

function assert(cond: unknown, msg: string): asserts cond {
    if (!cond) throw new Error(`smoke-test failed: ${msg}`);
}

function makeNode(id: string, data: Record<string, unknown>, type: string = 'default'): Node {
    return {
        id,
        type,
        position: { x: 0, y: 0 },
        data,
    } as unknown as Node;
}

function makeEdge(id: string, source: string, target: string): Edge {
    return { id, source, target } as unknown as Edge;
}

// Minimal graph: start -> action -> wait -> end
// Covers:
// - wait.match.action_id inference from incoming action
// - catalog defaults merged into action.with
const nodes: Node[] = [
    makeNode('start', { ntx_node_type: 'start', label: 'start' }, 'start'),
    makeNode(
        'a1',
        {
            ntx_node_type: 'action',
            label: 'udp#1',
            action_ref: 'udp#1',
            call: 'udp-send-reply',
            with: { target: 'udp-target' },
        },
        'action'
    ),
    makeNode(
        'w1',
        {
            ntx_node_type: 'wait',
            label: 'wait',
            on: { event: 'packet-rx', match: {} },
        },
        'wait'
    ),
    makeNode('end', { ntx_node_type: 'end', label: 'end' }, 'end'),
];

const edges: Edge[] = [
    makeEdge('e1', 'start', 'a1'),
    makeEdge('e2', 'a1', 'w1'),
    makeEdge('e3', 'w1', 'end'),
];

const issues = validateGraph(nodes, edges);
assert(issues.every((i) => i.level !== 'error'), `expected no errors, got: ${JSON.stringify(issues)}`);

const fakeCatalog = {
    schema_version: 1,
    actions: [
        {
            summary: { id: 'udp-send-reply' },
            spec: {
                summary: { id: 'udp-send-reply' },
                defaults_json: JSON.stringify({ payload_utf8: 'hello-from-catalog', timeout_ms: 1234 }),
            },
        },
    ],
} as any;

const exportBlocks = validateExportBlocking(nodes, edges, { catalog: fakeCatalog });
assert(exportBlocks.every((i) => i.level !== 'error'), `expected no export-blocking errors, got: ${JSON.stringify(exportBlocks)}`);

const scaffoldText = `{
  "version": "v1",
  "name": "template-name",
  "workbook": { "resources": [{ "id": "r1", "type": "demo", "properties": { "x": 1 } }] },
  "load": { "ramp_up": { "phases": [{ "at_second": 1, "spawn_users": 42 }] } },
  "user_resources": { "ip_binding": { "enabled": true, "pool_id": "default" } },
  "actions": { "actions": [{ "id": "old", "call": "old", "with": {} }] },
  "workflows": { "nodes": [{ "id": "old", "type": "end" }] }
}`;

// Parsed via YAML parser intentionally (YAML is a superset of JSON).
const scaffoldTemplate = parseYaml(scaffoldText) as any;

const yaml = buildScenarioYaml({
    workflowName: 'smoke',
    nodes,
    edges,
    options: { catalog: fakeCatalog, scaffold: scaffoldTemplate, includeDemoScaffold: false },
});
assert(typeof yaml === 'string' && yaml.length > 0, 'scenario.yaml should be non-empty');
assert(yaml.includes('version:'), 'scenario.yaml should contain version');
assert(yaml.includes('workflows:'), 'scenario.yaml should contain workflows');
assert(yaml.includes('actions:'), 'scenario.yaml should contain actions');

// Inferred wait match should include action_id udp#1
assert(yaml.includes('action_id:'), 'scenario.yaml should include inferred wait.on.match.action_id');
assert(yaml.includes('udp#1'), 'scenario.yaml should mention inferred action_id udp#1');

// Catalog defaults should be present (unless overridden)
assert(yaml.includes('payload:'), 'scenario.yaml should include catalog-derived payload');
assert(yaml.includes('hello-from-catalog'), 'scenario.yaml should include catalog default payload');
assert(yaml.includes('timeout_ms:'), 'scenario.yaml should include catalog-derived timeout_ms');

// Scaffold merge: keep workbook/load/user_resources from template.
assert(yaml.includes('workbook:'), 'scenario.yaml should include workbook from scaffold template');
assert(yaml.includes('resources:'), 'scenario.yaml should include workbook.resources from scaffold template');
assert(yaml.includes('r1'), 'scenario.yaml should include resource id from scaffold template');
assert(yaml.includes('load:'), 'scenario.yaml should include load from scaffold template');
assert(yaml.includes('spawn_users:'), 'scenario.yaml should include load.ramp_up from scaffold template');

console.log('workflow smoke OK');

// Negative case: wait node with no incoming action edge and no explicit match.action_id should block export.
const badNodes: Node[] = [
    makeNode('start', { ntx_node_type: 'start', label: 'start' }, 'start'),
    makeNode(
        'w1',
        {
            ntx_node_type: 'wait',
            label: 'wait',
            on: { event: 'packet-rx', match: {} },
        },
        'wait'
    ),
    makeNode('end', { ntx_node_type: 'end', label: 'end' }, 'end'),
];
const badEdges: Edge[] = [makeEdge('e1', 'start', 'w1'), makeEdge('e2', 'w1', 'end')];
const badBlocks = validateExportBlocking(badNodes, badEdges, { catalog: fakeCatalog });
assert(badBlocks.some((i) => i.level === 'error'), 'expected export-blocking error for wait without inferable action_id');
