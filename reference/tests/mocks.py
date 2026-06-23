"""Mock Neo4j 对象 — 供测试用例导入使用"""


class MockNeo4jNode:
    """模拟 Neo4j 节点 — 同时支持 dict-like 和 attribute 访问"""

    def __init__(self, node_id: str, labels: list[str], properties: dict | None = None):
        self._id = node_id
        self._labels = labels
        self._properties = properties or {}
        self.element_id = node_id
        self.labels = set(labels)

    def get(self, key, default=None):
        if key in ("node_id", "id"):
            return self._id
        if key == "label":
            return self._labels[0] if self._labels else "Unknown"
        return self._properties.get(key, default)

    def __iter__(self):
        all_items = {"node_id": self._id, "id": self._id,
                     "label": self._labels[0] if self._labels else "Unknown"}
        all_items.update(self._properties)
        return iter(all_items.items())

    def items(self):
        return list(self.__iter__())


class MockNeo4jRel:
    """模拟 Neo4j 关系"""

    def __init__(self, edge_id: str, rel_type: str, start_node: MockNeo4jNode,
                 end_node: MockNeo4jNode, properties: dict | None = None):
        self._id = edge_id
        self._type = rel_type
        self._properties = properties or {}
        self.element_id = edge_id
        self.start_node = start_node
        self.end_node = end_node
        self.type = rel_type

    def get(self, key, default=None):
        if key == "edge_id":
            return self._id
        if key == "relationship_type":
            return self._type
        if key == "relationship_name":
            return self._properties.get("relationship_name", self._type)
        return self._properties.get(key, default)

    def __iter__(self):
        all_items = {"edge_id": self._id, "relationship_type": self._type,
                     "relationship_name": self._properties.get("relationship_name", self._type)}
        all_items.update(self._properties)
        return iter(all_items.items())

    def items(self):
        return list(self.__iter__())


class MockNeo4jPath:
    """模拟 Neo4j Path"""

    def __init__(self, nodes: list[MockNeo4jNode], relationships: list[MockNeo4jRel]):
        self.nodes = nodes
        self.relationships = relationships
