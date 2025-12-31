import React from 'react';
import ReactDOM from 'react-dom/client';
import { ReactFlowProvider } from 'reactflow';
import { BrowserRouter } from 'react-router-dom';
import AppRouter from './AppRouter';
import './styles.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
        <BrowserRouter>
            <ReactFlowProvider>
                <AppRouter />
            </ReactFlowProvider>
        </BrowserRouter>
    </React.StrictMode>
);
