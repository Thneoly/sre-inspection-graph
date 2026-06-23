"""Inspection 相关模型"""

from pydantic import BaseModel
from typing import Optional
from datetime import datetime


class InspectionFindingOut(BaseModel):
    id: str
    rule_name: str
    severity: str
    status: str
    description: str
    detected_at: datetime
    recommendation: Optional[str] = None


class InspectionFindingsResponse(BaseModel):
    resource_id: str
    findings: list[InspectionFindingOut]


class InspectionRunOut(BaseModel):
    id: str
    run_name: str
    run_type: str
    overall_status: str
    started_at: datetime
    completed_at: Optional[datetime] = None
    total_rules: int
    passed_rules: int
    failed_rules: int
    skipped_rules: int


class InspectionRunsResponse(BaseModel):
    runs: list[InspectionRunOut]
    total: int
