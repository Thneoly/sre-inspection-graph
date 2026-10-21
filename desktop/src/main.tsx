import React from "react";
import ReactDOM from "react-dom/client";
import { QueryCache, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { HashRouter } from "react-router-dom";
import { ConfigProvider, message } from "antd";
import App from "./App";

// 全局 query 失败兜底:此前各页面只解构 data、无 isError 分支,后端 command
// 失败(如 SQLite 迁移错)页面静默空白。固定 key 让轮询类 query(connectors-status
// 每 5s)的连续失败只替换同一条消息,不堆叠刷屏。
const queryClient = new QueryClient({
  queryCache: new QueryCache({
    onError: (error, query) => {
      message.open({
        key: `query-error-${query.queryKey[0]}`,
        type: "error",
        content: `加载失败(${String(query.queryKey[0])}):${String(error)}`,
      });
    },
  }),
  defaultOptions: {
    queries: { refetchOnWindowFocus: false, retry: 1 },
  },
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <ConfigProvider>
        <HashRouter>
          <App />
        </HashRouter>
      </ConfigProvider>
    </QueryClientProvider>
  </React.StrictMode>
);
