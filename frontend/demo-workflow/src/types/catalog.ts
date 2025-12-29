export type ActionSummary = {
    id: string;
    title?: string | null;
    description?: string | null;
};

export type ActionSpec = {
    summary: ActionSummary;
    input_schema_json?: string | null;
    // Compatibility:
    // - demo-workflow initially used snake_case from a hand-written catalog.
    // - actions-catalog-gen emits kebab-case (default-params-json) and (params-schema-json).
    defaults_json?: string | null;
    default_params_json?: string | null;
    capabilities?: string[] | null;
    executor_version?: string | null;
};

export type ActionsCatalog = {
    // Same compatibility story: generator uses `schema-version`.
    schema_version?: number;
    schema_version_kebab?: number;
    // Prefer the generator shape in new code.
    'schema-version'?: number;
    generated_at?: string | null;
    executor_component?: {
        name?: string | null;
        version?: string | null;
        digest?: string | null;
    } | null;
    actions: Array<{ summary: ActionSummary; spec?: ActionSpec | null }>;
};
