import { useMemo, useState } from 'react';
import { Link, useLocation, useNavigate } from 'react-router-dom';

import { getAuthUser, login } from '../auth';

type LoginLocationState = {
    from?: string;
};

export function LoginPage() {
    const nav = useNavigate();
    const location = useLocation();

    const already = getAuthUser();

    const [user, setUser] = useState<string>(already ?? '');
    const [error, setError] = useState<string | null>(null);

    const from = useMemo(() => {
        const st = (location.state ?? {}) as LoginLocationState;
        return typeof st.from === 'string' && st.from.trim() ? st.from : '/';
    }, [location.state]);

    function onSubmit(e: React.FormEvent) {
        e.preventDefault();
        const trimmed = user.trim();
        if (!trimmed) {
            setError('Please enter a username');
            return;
        }
        login(trimmed);
        nav(from, { replace: true });
    }

    return (
        <div style={{ padding: 16, maxWidth: 520 }}>
            <h1 style={{ margin: 0, fontSize: 18 }}>Login</h1>
            <div style={{ marginTop: 8, color: '#666', fontSize: 12 }}>
                Enter any username for this demo.
            </div>

            <form onSubmit={onSubmit} style={{ marginTop: 12, display: 'flex', gap: 8, alignItems: 'center' }}>
                <input
                    style={{ flex: 1, padding: '8px 10px' }}
                    value={user}
                    onChange={(e) => {
                        setUser(e.target.value);
                        setError(null);
                    }}
                    placeholder="username"
                    autoFocus
                />
                <button type="submit">Login</button>
            </form>

            {error ? <div style={{ marginTop: 10, color: 'crimson' }}>{error}</div> : null}

            <div style={{ marginTop: 12, fontSize: 12 }}>
                <Link to="/">Go to Home</Link>
            </div>
        </div>
    );
}
