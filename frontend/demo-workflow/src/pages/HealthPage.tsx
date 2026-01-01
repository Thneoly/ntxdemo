export function HealthPage() {
    return (
        <div className="page">
            <h1 className="pageTitleBlock">
                Health
            </h1>
            <div className="card" style={{ maxWidth: 420 }}>
                <div style={{ fontWeight: 600 }}>Status</div>
                <div className="muted" style={{ marginTop: 6 }}>
                    ok
                </div>
            </div>
        </div>
    );
}
