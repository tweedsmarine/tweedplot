-- TweedPlot's side of the S-100 Part 9a portrayal API (the "Host" functions the IHO
-- portrayal rules call). The data comes from Rust:
--   HostFC: the feature catalogue as plain tables (built once)
--   HostDS: the features being portrayed (set before each run)
--   HostEmitted: filled with { id, instructions, observed } by HostPortrayalEmit
-- TweedPlot code (GPL-3.0-or-later), not part of the IHO catalogue.

local fc = HostFC

local function codes(t)
	local out = {}
	for code in pairs(t) do out[#out + 1] = code end
	table.sort(out)
	return out
end

function HostGetFeatureTypeCodes() return codes(fc.features) end
function HostGetInformationTypeCodes() return codes(fc.information) end
function HostGetSimpleAttributeTypeCodes() return codes(fc.simple) end
function HostGetComplexAttributeTypeCodes() return codes(fc.complex) end
function HostGetRoleTypeCodes() return fc.roles end
function HostGetInformationAssociationTypeCodes() return codes(fc.iassoc) end
function HostGetFeatureAssociationTypeCodes() return codes(fc.fassoc) end

local function attributeBindings(list)
	local out = {}
	for _, b in ipairs(list) do
		out[#out + 1] = CreateAttributeBinding(b.code, b.lower, b.upper, b.sequential, b.permitted)
	end
	return out
end

local function item(t)
	return CreateItem(t.code, t.name, '', nil, t.aliases)
end

function HostGetFeatureTypeInfo(code)
	local f = fc.features[code]
	if not f then return nil end
	local named = CreateNamedType(item(f), f.abstract, attributeBindings(f.bindings))
	return CreateFeatureType(CreateObjectType(named, {}), f.useType, f.primitives, {}, nil, nil)
end

function HostGetInformationTypeInfo(code)
	local t = fc.information[code]
	if not t then return nil end
	local named = CreateNamedType(item(t), false, attributeBindings(t.bindings))
	return CreateInformationType(CreateObjectType(named, {}), nil, nil)
end

function HostGetSimpleAttributeTypeInfo(code)
	local a = fc.simple[code]
	if not a then return nil end
	local listed = {}
	for _, v in ipairs(a.listed) do
		listed[#listed + 1] = CreateListedValue(v.label, '', v.code, nil, nil)
	end
	return CreateSimpleAttribute(item(a), a.valueType, nil, nil, nil, listed)
end

function HostGetComplexAttributeTypeInfo(code)
	local a = fc.complex[code]
	if not a then return nil end
	return CreateComplexAttribute(item(a), attributeBindings(a.bindings))
end

-- Features --------------------------------------------------------------------

function HostGetFeatureIDs() return HostDS.ids end

function HostFeatureGetCode(featureID) return HostDS.features[featureID].code end

-- The attribute container at a path "code:index;code:index" (complex attributes).
local function container(attrs, path)
	if path == nil or path == '' then return attrs end
	for step in string.gmatch(path, '[^;]+') do
		local code, index = string.match(step, '^(.-):(%d+)$')
		local list = attrs and attrs[code]
		attrs = list and list[tonumber(index)]
		if not attrs then return nil end
	end
	return attrs
end

-- Simple attribute values are kept as arrays of strings; complex ones as arrays of tables.
function HostFeatureGetSimpleAttribute(featureID, path, code)
	local c = container(HostDS.features[featureID].attrs, path)
	return (c and c[code]) or {}
end

function HostFeatureGetComplexAttributeCount(featureID, path, code)
	local c = container(HostDS.features[featureID].attrs, path)
	return (c and c[code] and #c[code]) or 0
end

-- also the generic forms some rules use
HostGetSimpleAttribute = HostFeatureGetSimpleAttribute
HostGetComplexAttributeCount = HostFeatureGetComplexAttributeCount

function HostFeatureGetSpatialAssociations(featureID)
	local f = HostDS.features[featureID]
	if not f.spatial then return {} end
	return { CreateSpatialAssociation(f.spatialType, f.spatial, 'Forward') }
end

function HostFeatureGetAssociatedFeatureIDs(featureID, associationCode, roleCode) return {} end
function HostFeatureGetAssociatedInformationIDs(featureID, associationCode, roleCode) return {} end
function HostSpatialGetAssociatedFeatureIDs(spatialID) return HostDS.spatialFeatures[spatialID] or {} end
function HostSpatialGetAssociatedInformationIDs(spatialID, associationCode, roleCode) return {} end
function HostInformationTypeGetCode(informationID) return nil end
function HostInformationTypeGetSimpleAttribute() return {} end
function HostInformationTypeGetComplexAttributeCount() return 0 end

-- Spatial objects: points { x, y }, curves { points = { {x,y}, ... } }, surfaces { rings = { curveID, ... } }
local function pointAssociation(id, p)
	local pid = id .. '#' .. p[1] .. ',' .. p[2]
	HostDS.spatials[pid] = { kind = 'Point', x = p[1], y = p[2] }
	return CreateSpatialAssociation('Point', pid, 'Forward')
end

function HostGetSpatial(spatialID)
	local s = HostDS.spatials[spatialID]
	if not s then return nil end
	if s.kind == 'Point' then
		return CreatePoint(tostring(s.x), tostring(s.y))
	elseif s.kind == 'MultiPoint' then
		local pts = {}
		for _, p in ipairs(s.points) do pts[#pts + 1] = CreatePoint(tostring(p[1]), tostring(p[2]), p[3] and tostring(p[3]) or nil) end
		return CreateMultiPoint(pts)
	elseif s.kind == 'Curve' then
		local pts = {}
		for _, p in ipairs(s.points) do pts[#pts + 1] = CreatePoint(tostring(p[1]), tostring(p[2])) end
		local first, last = s.points[1], s.points[#s.points]
		return CreateCurve(pointAssociation(spatialID, first), pointAssociation(spatialID, last), { CreateCurveSegment(pts, 'Loxodromic') })
	elseif s.kind == 'Surface' then
		local rings = {}
		for i, r in ipairs(s.rings) do rings[i] = CreateSpatialAssociation('Curve', r, 'Forward') end
		local exterior = table.remove(rings, 1)
		return CreateSurface(exterior, rings)
	end
end

function HostPortrayalEmit(featureReference, drawingInstructions, observedParameters)
	HostEmitted[#HostEmitted + 1] = { id = featureReference, instructions = drawingInstructions, observed = observedParameters }
	return true
end

function HostDebuggerEntry(kind, message)
	if kind == 'trace' or kind == 'first_chance_error' then
		HostTrace[#HostTrace + 1] = tostring(message)
	end
end
