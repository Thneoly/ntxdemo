export function HealthPage() {
    return (
        <div className="page">
            <div className="pageHeader">
                <h1 className="pageTitle">Health</h1>
            </div>
            <div className="card" style={{ maxWidth: 420 }}>
                <div style={{ fontWeight: 600 }}>Status</div>
                <div className="muted" style={{ marginTop: 6 }}>
                    ok
                </div>
            </div>
        </div>
    );
}
