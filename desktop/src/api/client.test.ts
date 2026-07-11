import { describe, expect, it, vi, beforeEach } from "vitest";

// Mock @tauri-apps/api/core 的 invoke,测 api/client 包装层(命令名 + 参数形状)。
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  recordChangeEvent, executeRecovery, listRecoveryExecutions, listChainTemplates,
  recordAlert, changeEventRecoverySuggestion,
} from "./client";

const mockedInvoke = invoke as unknown as ReturnType<typeof vi.fn>;

beforeEach(() => {
  mockedInvoke.mockReset();
});

describe("api/client invoke wrappers (Phase 3.6)", () => {
  it("recordChangeEvent sends record_change_event with req payload", async () => {
    mockedInvoke.mockResolvedValueOnce({ change_event_id: "ce-x" });
    await recordChangeEvent({ change_type: "configmap_updated", target_resource_id: "cm:a" });
    expect(mockedInvoke).toHaveBeenCalledWith("record_change_event", {
      req: { change_type: "configmap_updated", target_resource_id: "cm:a" },
    });
  });

  it("executeRecovery sends execute_recovery with camelCase args", async () => {
    mockedInvoke.mockResolvedValueOnce({ execution_id: "e1", status: "succeeded" });
    await executeRecovery({
      actionId: "scale_deployment",
      targetResourceId: "deploy:a",
      inputParams: { replicas_delta: 1 },
    });
    expect(mockedInvoke).toHaveBeenCalledWith(
      "execute_recovery",
      expect.objectContaining({
        actionId: "scale_deployment",
        targetResourceId: "deploy:a",
        inputParams: { replicas_delta: 1 },
      })
    );
  });

  it("listRecoveryExecutions sends list_recovery_executions with empty opts when none given", async () => {
    mockedInvoke.mockResolvedValueOnce([]);
    await listRecoveryExecutions();
    expect(mockedInvoke).toHaveBeenCalledWith("list_recovery_executions", {});
  });

  it("listRecoveryExecutions forwards filter opts as-is", async () => {
    mockedInvoke.mockResolvedValueOnce([]);
    await listRecoveryExecutions({ status: "awaiting_approval", limit: 10 });
    expect(mockedInvoke).toHaveBeenCalledWith("list_recovery_executions", {
      status: "awaiting_approval",
      limit: 10,
    });
  });

  it("listChainTemplates calls list_chain_templates with no args", async () => {
    mockedInvoke.mockResolvedValueOnce([]);
    await listChainTemplates();
    expect(mockedInvoke).toHaveBeenCalledWith("list_chain_templates");
  });

  it("recordAlert sends record_alert with req payload", async () => {
    mockedInvoke.mockResolvedValueOnce({ alert_event_id: "a1" });
    await recordAlert({ alert_name: "HighErr", resource_ref: "svc:a" });
    expect(mockedInvoke).toHaveBeenCalledWith("record_alert", {
      req: { alert_name: "HighErr", resource_ref: "svc:a" },
    });
  });

  it("changeEventRecoverySuggestion sends change_event_recovery_suggestion", async () => {
    mockedInvoke.mockResolvedValueOnce({ suggestions: [], total: 0 });
    await changeEventRecoverySuggestion("ce-1");
    expect(mockedInvoke).toHaveBeenCalledWith("change_event_recovery_suggestion", {
      changeEventId: "ce-1",
    });
  });
});
