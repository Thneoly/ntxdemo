export function yamlScalar(value: unknown): string {
    if (value === null || value === undefined) return 'null';
    if (typeof value === 'number' || typeof value === 'boolean') return String(value);
    // Quote strings by default to avoid surprises.
    const s = String(value);
    const escaped = s.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
    return `"${escaped}"`;
}

const KEY_ORDER: Record<string, string[]> = {
    // Scenario root (matches component/conf/udp-echo-minimal/scenario_demo.yaml)
    '': ['version', 'name', 'workbook', 'load', 'user_resources', 'actions', 'workflows'],

    // workbook
    workbook: ['resources'],
    resources: ['id', 'type', 'properties'],

    // load
    load: ['ramp_up', 'user_lifetime'],
    ramp_up: ['phases'],
    user_lifetime: ['mode', 'max_concurrency'],
    phases: ['at_second', 'spawn_users'],

    // user_resources / ip_binding
    user_resources: ['ip_binding'],
    ip_binding: ['enabled', 'pool_id'],

    // actions
    actions: ['actions'],
    // An action definition object
    action: ['id', 'call', 'with'],

    // workflows
    workflows: ['nodes'],
    // A workflow node object (prefer demo order)
    node: ['id', 'type', 'priority', 'action', 'on', 'edges'],
    on: ['event', 'match'],
    match: ['action_id'],
    edges: ['to', 'label'],
};

function isPlainObject(v: unknown): v is Record<string, unknown> {
    return Boolean(v && typeof v === 'object' && !Array.isArray(v));
}

function orderedKeys(obj: Record<string, unknown>, ctx: string): string[] {
    const keys = Object.keys(obj);
    const preferred = KEY_ORDER[ctx] ?? [];
    const preferredSet = new Set(preferred);
    const first = preferred.filter((k) => keys.includes(k));
    const rest = keys
        .filter((k) => !preferredSet.has(k))
        .sort((a, b) => a.localeCompare(b));
    return [...first, ...rest];
}

function nextCtx(parentCtx: string, key: string, value: unknown): string {
    // Root -> scenario
    if (parentCtx === '') {
        return key;
    }

    // Arrays where the *item* should have its own ordering context
    if (key === 'resources' && Array.isArray(value)) return 'resources';
    if (key === 'actions' && Array.isArray(value)) return 'action';
    if (key === 'nodes' && Array.isArray(value)) return 'node';
    if (key === 'edges' && Array.isArray(value)) return 'edges';
    if (key === 'phases' && Array.isArray(value)) return 'phases';

    return key;
}

export function yamlObject(obj: Record<string, unknown>, indent: number): string {
    // Backwards-compatible wrapper: default to root ctx.
    return yamlObjectWithCtx(obj, indent, '');
}

function yamlObjectWithCtx(obj: Record<string, unknown>, indent: number, ctx: string): string {
    const pad = ' '.repeat(indent);
    const rootExtraSpacing = indent === 0 && ctx === '';
    const parts = orderedKeys(obj, ctx).map((k) => {
        const v = obj[k];
        if (isPlainObject(v)) {
            const childCtx = nextCtx(ctx, k, v);
            return `${pad}${k}:\n${yamlObjectWithCtx(v as Record<string, unknown>, indent + 2, childCtx)}`;
        }
        if (Array.isArray(v)) {
            if (v.length === 0) return `${pad}${k}: []`;
            const itemCtx = nextCtx(ctx, k, v);
            return [
                `${pad}${k}:`,
                ...v.map((item) => {
                    if (isPlainObject(item)) {
                        return `${pad}  -\n${yamlObjectWithCtx(item as Record<string, unknown>, indent + 4, itemCtx)}`;
                    }
                    return `${pad}  - ${yamlScalar(item)}`;
                }),
            ].join('\n');
        }
        return `${pad}${k}: ${yamlScalar(v)}`;
    });

    // Add a blank line between top-level sections for readability.
    return parts.join(rootExtraSpacing ? '\n\n' : '\n');
}
