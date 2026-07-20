import { describe, expect, it, vi, beforeEach } from "vitest";

// Mock @tauri-apps/api/core 的 invoke,测 api/client 包装层(命令名 + 参数形状)。
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  recordChangeEvent, executeRecovery, listRecoveryExecutions, listChainTemplates,
  recordAlert, changeEventRecoverySuggestion,
  generateReport, listReports, createSubscription, triggerSubscriptionNow, listSentEmails,
  clearReports,
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

  it("generateReport sends generate_report_cmd with camelCase opts (Phase 4.1)", async () => {
    mockedInvoke.mockResolvedValueOnce({ report_id: "rpt-1", status: "completed" });
    await generateReport({ templateId: "application_health", applicationId: "app:order" });
    expect(mockedInvoke).toHaveBeenCalledWith("generate_report_cmd", {
      templateId: "application_health",
      applicationId: "app:order",
    });
  });

  it("listReports sends list_reports with empty opts when none given (Phase 4.1)", async () => {
    mockedInvoke.mockResolvedValueOnce([]);
    await listReports();
    expect(mockedInvoke).toHaveBeenCalledWith("list_reports", {});
  });

  it("createSubscription sends create_subscription with cron + recipients (Phase 4.3)", async () => {
    mockedInvoke.mockResolvedValueOnce({ subscription_id: "sub-1" });
    await createSubscription({
      templateId: "cluster_overview",
      clusterId: "otel-demo",
      cron: "0 9 * * 1",
      recipients: ["ops@example.com"],
    });
    expect(mockedInvoke).toHaveBeenCalledWith("create_subscription", {
      templateId: "cluster_overview",
      clusterId: "otel-demo",
      cron: "0 9 * * 1",
      recipients: ["ops@example.com"],
    });
  });

  it("triggerSubscriptionNow sends trigger_subscription_now (Phase 4.3)", async () => {
    mockedInvoke.mockResolvedValueOnce({ report_id: "rpt-2", status: "completed" });
    await triggerSubscriptionNow("sub-1");
    expect(mockedInvoke).toHaveBeenCalledWith("trigger_subscription_now", {
      subscriptionId: "sub-1",
    });
  });

  it("listSentEmails sends list_sent_emails with no args (Phase 4.3)", async () => {
    mockedInvoke.mockResolvedValueOnce([]);
    await listSentEmails();
    expect(mockedInvoke).toHaveBeenCalledWith("list_sent_emails");
  });

  it("clearReports sends clear_reports with no args (Phase 4.3 后续)", async () => {
    mockedInvoke.mockResolvedValueOnce(3);
    await clearReports();
    expect(mockedInvoke).toHaveBeenCalledWith("clear_reports");
  });
});
