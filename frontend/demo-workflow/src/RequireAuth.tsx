import type { ReactNode } from 'react';
import { Navigate, Outlet, useLocation } from 'react-router-dom';

import { isLoggedIn } from './auth';

export function RequireAuth(props: { children: ReactNode }) {
    const location = useLocation();

    if (!isLoggedIn()) {
        return <Navigate to="/login" replace state={{ from: location.pathname + location.search }} />;
    }

    return props.children;
}

export function RequireAuthOutlet() {
    return (
        <RequireAuth>
            <Outlet />
        </RequireAuth>
    );
}
