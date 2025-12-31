import { Navigate, Route, Routes } from 'react-router-dom';

import App from './App';
import { RequireAuth } from './RequireAuth';
import { HealthPage } from './pages/HealthPage';
import { HomePage } from './pages/HomePage';
import { LoginPage } from './pages/LoginPage';
import { WasmListPage } from './pages/WasmListPage';
import { WasmUploadPage } from './pages/WasmUploadPage';

export default function AppRouter() {
    return (
        <Routes>
            <Route path="/login" element={<LoginPage />} />

            <Route
                path="/"
                element={
                    <RequireAuth>
                        <HomePage />
                    </RequireAuth>
                }
            />
            <Route
                path="/builder"
                element={
                    <RequireAuth>
                        <App />
                    </RequireAuth>
                }
            />
            <Route
                path="/health"
                element={
                    <RequireAuth>
                        <HealthPage />
                    </RequireAuth>
                }
            />
            <Route
                path="/wasm"
                element={
                    <RequireAuth>
                        <WasmListPage />
                    </RequireAuth>
                }
            />
            <Route
                path="/wasm/upload"
                element={
                    <RequireAuth>
                        <WasmUploadPage />
                    </RequireAuth>
                }
            />
            <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
    );
}
