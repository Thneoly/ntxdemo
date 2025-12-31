import type { ReactElement } from 'react';
import { Navigate, useLocation } from 'react-router-dom';

import { isLoggedIn } from './auth';

export function RequireAuth(props: { children: ReactElement }) {
    const location = useLocation();

    if (!isLoggedIn()) {
        return <Navigate to="/login" replace state={{ from: location.pathname + location.search }} />;
    }

    return props.children;
}
