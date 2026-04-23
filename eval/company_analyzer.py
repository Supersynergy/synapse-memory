#!/usr/bin/env python3
"""
company_analyzer.py

Company Intelligence Analysis for Synapse Brain.
"""

import sqlite3
import json
import os
from pathlib import Path
from dataclasses import dataclass, field
from typing import List, Dict, Optional
from datetime import datetime
from collections import defaultdict
import re

BRAIN_DB = os.path.expanduser("~/.synapse/brain.db")

@dataclass
class CompanyAnalysis:
    timestamp: datetime = field(default_factory=datetime.now)
    org_chart: Dict = field(default_factory=dict)
    tech_stack: Dict = field(default_factory=dict)
    data_flows: List[Dict] = field(default_factory=list)
    people: List[Dict] = field(default_factory=list)
    products: List[Dict] = field(default_factory=list)
    processes: List[Dict] = field(default_factory=list)
    metrics: Dict = field(default_factory=dict)
    risks: List[Dict] = field(default_factory=list)
    recommendations: List[str] = field(default_factory=list)

class CompanyAnalyzer:
    def __init__(self, db_path: str = BRAIN_DB):
        self.db_path = db_path
        self.con = sqlite3.connect(db_path)
        self.analysis = CompanyAnalysis()
        
    def close(self):
        self.con.close()
    
    def analyze_all(self) -> CompanyAnalysis:
        print("Running company analysis...")
        self.analyze_org_structure()
        self.analyze_tech_stack()
        self.analyze_data_flows()
        self.analyze_products()
        self.analyze_processes()
        self.analyze_metrics()
        self.assess_risks()
        self.generate_recommendations()
        return self.analysis
    
    def analyze_org_structure(self):
        print("  Analyzing org structure...")
        teams = defaultdict(list)
        cursor = self.con.execute("SELECT title, text FROM docs LIMIT 10000")
        for title, text in cursor.fetchall():
            if not text:
                continue
            patterns = [
                r'team[:\s]+([A-Za-z]+)',
                r'department[:\s]+([A-Za-z]+)',
                r'owned by[:\s]+([A-Za-z]+)',
            ]
            for pattern in patterns:
                matches = re.findall(pattern, text.lower())
                for match in matches:
                    teams[match].append(title)
        self.analysis.org_chart = {
            "teams": {k: len(v) for k, v in teams.items()},
            "total_teams": len(teams),
            "top_teams": sorted(teams.items(), key=lambda x: -len(x[1]))[:10],
        }
    
    def analyze_tech_stack(self):
        print("  Analyzing tech stack...")
        tech_counts = defaultdict(int)
        languages = {
            "python": r'\.py\b',
            "javascript": r'\.js\b',
            "typescript": r'\.ts\b',
            "java": r'\.java\b',
            "go": r'\.go\b',
            "rust": r'\.rs\b',
            "ruby": r'\.rb\b',
        }
        frameworks = {
            "react": r'\breact\b',
            "django": r'\bdjango\b',
            "flask": r'\bflask\b',
            "fastapi": r'\bfastapi\b',
            "express": r'\bexpress\b',
        }
        databases = {
            "postgresql": r'\bpostgresql\b|\bpostgres\b',
            "mysql": r'\bmysql\b',
            "mongodb": r'\bmongodb\b',
            "redis": r'\bredis\b',
        }
        cursor = self.con.execute("SELECT text FROM docs LIMIT 50000")
        for (text,) in cursor.fetchall():
            if not text:
                continue
            text_lower = text.lower()
            for lang, pattern in languages.items():
                if re.search(pattern, text_lower):
                    tech_counts[f"lang:{lang}"] += 1
            for fw, pattern in frameworks.items():
                if re.search(pattern, text_lower):
                    tech_counts[f"framework:{fw}"] += 1
            for db, pattern in databases.items():
                if re.search(pattern, text_lower):
                    tech_counts[f"database:{db}"] += 1
        self.analysis.tech_stack = {
            "languages": {k.replace("lang:", ""): v for k, v in tech_counts.items() if k.startswith("lang:")},
            "frameworks": {k.replace("framework:", ""): v for k, v in tech_counts.items() if k.startswith("framework:")},
            "databases": {k.replace("database:", ""): v for k, v in tech_counts.items() if k.startswith("database:")},
            "total_technologies": len(tech_counts),
        }
    
    def analyze_data_flows(self):
        print("  Analyzing data flows...")
        flows = []
        cursor = self.con.execute("SELECT title, text FROM docs LIMIT 10000")
        for title, text in cursor.fetchall():
            if not text:
                continue
            api_patterns = [
                r'@app\.(get|post|put|delete)\(["\']([^"\']+)["\']',
                r'app\.(get|post|put|delete)\(["\']([^"\']+)["\']',
            ]
            for pattern in api_patterns:
                matches = re.findall(pattern, text)
                for method, path in matches:
                    flows.append({"type": "api", "method": method.upper(), "path": path, "source": title})
        self.analysis.data_flows = flows[:1000]
    
    def analyze_products(self):
        print("  Analyzing products...")
        products = []
        cursor = self.con.execute("SELECT title, text FROM docs LIMIT 10000")
        for title, text in cursor.fetchall():
            if not text:
                continue
            patterns = [
                r'service[:\s]+([A-Za-z0-9_-]+)',
                r'product[:\s]+([A-Za-z0-9_-]+)',
                r'project[:\s]+([A-Za-z0-9_-]+)',
            ]
            for pattern in patterns:
                matches = re.findall(pattern, text.lower())
                for match in matches:
                    products.append({"name": match, "source": title})
        seen = set()
        unique = []
        for p in products:
            key = (p['name'], p['source'])
            if key not in seen:
                seen.add(key)
                unique.append(p)
        self.analysis.products = unique[:500]
    
    def analyze_processes(self):
        print("  Analyzing processes...")
        processes = []
        cursor = self.con.execute("SELECT title, text FROM docs LIMIT 10000")
        for title, text in cursor.fetchall():
            if not text:
                continue
            if re.search(r'github actions|gitlab ci|jenkins', text, re.I):
                processes.append({"type": "cicd", "name": title})
            if re.search(r'deploy|kubernetes|docker', text, re.I):
                processes.append({"type": "deployment", "name": title})
        self.analysis.processes = processes[:500]
    
    def analyze_metrics(self):
        print("  Analyzing metrics...")
        self.analysis.metrics = {
            "total_documents": self.con.execute("SELECT COUNT(*) FROM docs").fetchone()[0],
            "total_size_bytes": self.con.execute("SELECT SUM(LENGTH(text)) FROM docs").fetchone()[0] or 0,
        }
    
    def assess_risks(self):
        print("  Assessing risks...")
        risks = []
        cursor = self.con.execute("SELECT title, text FROM docs LIMIT 10000")
        for title, text in cursor.fetchall():
            if not text:
                continue
            if re.search(r'password\s*=\s*["\'][^"\']+["\']', text):
                risks.append({"severity": "critical", "category": "security", "description": "Hardcoded password", "source": title})
            if re.search(r'api[_-]?key\s*=\s*["\'][^"\']+["\']', text, re.I):
                risks.append({"severity": "high", "category": "security", "description": "Hardcoded API key", "source": title})
        self.analysis.risks = risks[:50]
    
    def generate_recommendations(self):
        recommendations = []
        if self.analysis.tech_stack:
            top_langs = sorted(self.analysis.tech_stack.get("languages", {}).items(), key=lambda x: -x[1])[:5]
            if top_langs:
                recommendations.append(f"Primary languages: {', '.join(l[0] for l in top_langs)}")
        critical_risks = [r for r in self.analysis.risks if r.get("severity") == "critical"]
        if critical_risks:
            recommendations.append(f"Address {len(critical_risks)} critical security issues")
        self.analysis.recommendations = recommendations
    
    def export_report(self, format: str = "json") -> str:
        if format == "json":
            return json.dumps(self.analysis.__dict__, default=str, indent=2)
        elif format == "markdown":
            md = ["# Company Analysis Report", f"\nGenerated: {self.analysis.timestamp}\n"]
            md.append("## Organization Structure")
            md.append(f"- Total Teams: {self.analysis.org_chart.get('total_teams', 0)}")
            md.append("\n## Technology Stack")
            tech = self.analysis.tech_stack
            if tech:
                md.append(f"\nLanguages ({len(tech.get('languages', {}))}):")
                for lang, count in list(tech.get('languages', {}).items())[:10]:
                    md.append(f"- {lang}: {count}")
            md.append("\n## Metrics")
            md.append(f"- Total Documents: {self.analysis.metrics.get('total_documents', 0):,}")
            if self.analysis.risks:
                md.append("\n## Risks")
                for risk in self.analysis.risks[:10]:
                    md.append(f"- [{risk.get('severity', '?').upper()}] {risk.get('description', '')}")
            if self.analysis.recommendations:
                md.append("\n## Recommendations")
                for rec in self.analysis.recommendations:
                    md.append(f"- {rec}")
            return "\n".join(md)
        raise ValueError(f"Unknown format: {format}")

def main():
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", default=BRAIN_DB)
    parser.add_argument("--format", choices=["json", "markdown"], default="markdown")
    parser.add_argument("--output")
    args = parser.parse_args()
    
    analyzer = CompanyAnalyzer(args.db)
    analysis = analyzer.analyze_all()
    report = analyzer.export_report(args.format)
    
    if args.output:
        with open(args.output, "w") as f:
            f.write(report)
        print(f"Report saved to {args.output}")
    else:
        print(report)
    
    analyzer.close()

if __name__ == "__main__":
    main()
