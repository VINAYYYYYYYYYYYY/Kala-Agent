// Narrow C ABI adapter over OpenCascade for the native `kala-cad` crate.
//
// Rules enforced here:
//   * every exported function catches Standard_Failure, std::exception and
//     every other exception; nothing crosses the C ABI;
//   * every pointer is owned by exactly one side: shapes/meshes/buffers are
//     heap objects created here and released by the matching `*_free`;
//   * the adapter never formats product/contract messages; it only reports a
//     status code plus a bounded exception class name so Rust owns all
//     user-visible wording;
//   * no file paths or host filesystem access: STEP is read/written through
//     in-memory std::stream objects.
//
// Product semantics (planes, units in the script, cut directions, budgets,
// aliases) live in Rust. This file lowers already-computed points, vectors and
// tolerances to TopoDS shapes and reads back kernel measurements.

#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepAlgoAPI_Section.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepBndLib.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepTools_WireExplorer.hxx>
#include <BRep_Builder.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GProp_GProps.hxx>
#include <GCPnts_QuasiUniformDeflection.hxx>
#include <HLRAlgo_Projector.hxx>
#include <HLRBRep_Algo.hxx>
#include <HLRBRep_HLRToShape.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <Interface_Static.hxx>
#include <Poly_Triangulation.hxx>
#include <ShapeAnalysis_FreeBounds.hxx>
#include <Standard_Failure.hxx>
#include <Standard_Version.hxx>
#include <STEPControl_Controller.hxx>
#include <STEPControl_Reader.hxx>
#include <STEPControl_StepModelType.hxx>
#include <STEPControl_Writer.hxx>
#include <TopAbs.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopLoc_Location.hxx>
#include <TopTools_HSequenceOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Face.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Circ.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <exception>
#include <istream>
#include <memory>
#include <new>
#include <ostream>
#include <sstream>
#include <string>
#include <tuple>
#include <vector>

namespace {

constexpr int32_t kOk = 0;
constexpr int32_t kNotDone = 1;
constexpr int32_t kNullResult = 2;
constexpr int32_t kException = 3;
constexpr int32_t kInvalidArgument = 4;
constexpr int32_t kOverVertexBudget = 5;
constexpr int32_t kOverTriangleBudget = 6;
constexpr int32_t kNoTriangles = 7;
constexpr int32_t kVoidBox = 8;
constexpr int32_t kNormalsFailed = 9;

constexpr std::size_t kErrorMessageSize = 1024;

struct KalaCadError {
    int32_t code;
    uint32_t message_len;
    char message[kErrorMessageSize];
};

struct KalaMeasurement {
    double volume;
    double area;
    double centroid[3];
    double min[3];
    double max[3];
    int64_t solid_count;
    int32_t valid;
    int32_t reserved;
};

struct KalaPublishState {
    int64_t solid_count;
    double volume;
    int32_t is_null;
    int32_t valid;
};

struct KalaTopology {
    int64_t solids;
    int64_t shells;
    int64_t faces;
    int64_t edges;
    int64_t vertices;
};

struct KalaMesh {
    std::vector<double> positions;
    std::vector<double> normals;
    std::vector<uint32_t> indices;
};

struct KalaBuffer {
    std::string data;
};

struct KalaPolyline {
    // Flat u,v pairs in view-plane millimetres.
    std::vector<double> coordinates;
    bool closed = false;
};

struct KalaDrawing {
    std::vector<KalaPolyline> visible;
    std::vector<KalaPolyline> hidden;
};

struct KalaLoop {
    TopoDS_Wire wire;
    std::vector<double> coordinates;
    double length = 0.0;
    double area = 0.0;
    bool area_measured = false;
    bool simple = true;
};

struct KalaSection {
    std::vector<KalaLoop> loops;
};

struct KalaSectionLoopInfo {
    int64_t point_count;
    double length;
    double area;
    int32_t area_measured;
    int32_t simple;
    int32_t reserved;
};

constexpr double kGeometryEpsilon = 1e-9;
constexpr double kSectionWireTolerance = 1e-6;

struct PlaneProjection {
    double u[3];
    double v[3];
};

void project_point(const PlaneProjection& projection, const gp_Pnt& point, double& u,
                   double& v) {
    u = point.X() * projection.u[0] + point.Y() * projection.u[1] +
        point.Z() * projection.u[2];
    v = point.X() * projection.v[0] + point.Y() * projection.v[1] +
        point.Z() * projection.v[2];
}

void clear_error(KalaCadError* error) {
    if (error == nullptr) {
        return;
    }
    error->code = kOk;
    error->message_len = 0;
    error->message[0] = '\0';
}

void set_error(KalaCadError* error, int32_t code, const char* text) {
    if (error == nullptr) {
        return;
    }
    error->code = code;
    const char* source = text != nullptr ? text : "";
    // Bounded scan: never read past the fixed transport buffer even if a
    // caller ever passes an unterminated or very long message.
    std::size_t length = 0;
    while (length < kErrorMessageSize && source[length] != '\0') {
        ++length;
    }
    if (length > 0) {
        std::memcpy(error->message, source, length);
    }
    error->message_len = static_cast<uint32_t>(length);
}

void set_exception(KalaCadError* error) {
    set_error(error, kException, "unknown");
}

const char* exception_class(const Standard_Failure& failure) {
    const Handle(Standard_Type)& type = failure.DynamicType();
    if (type.IsNull()) {
        return "Standard_Failure";
    }
    const char* name = type->Name();
    return name != nullptr ? name : "Standard_Failure";
}

const TopoDS_Shape* as_shape(const void* handle) {
    return static_cast<const TopoDS_Shape*>(handle);
}

TopoDS_Shape* as_shape(void* handle) {
    return static_cast<TopoDS_Shape*>(handle);
}

void* new_shape(const TopoDS_Shape& shape) {
    return new TopoDS_Shape(shape);
}

bool finite3(double x, double y, double z) {
    return std::isfinite(x) && std::isfinite(y) && std::isfinite(z);
}

// ---------------------------------------------------------------------------
// Version and shape lifetime
// ---------------------------------------------------------------------------

const char* impl_kala_occt_version() noexcept {
    return OCC_VERSION_COMPLETE;
}

void* impl_kala_shape_new_null(KalaCadError* error) noexcept {
    clear_error(error);
    try {
        return new TopoDS_Shape();
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return nullptr;
    } catch (const std::exception&) {
        set_exception(error);
        return nullptr;
    } catch (...) {
        set_exception(error);
        return nullptr;
    }
}

void* impl_kala_shape_copy(const void* handle, KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr) {
        set_error(error, kInvalidArgument, "shape");
        return nullptr;
    }
    try {
        return new TopoDS_Shape(*as_shape(handle));
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return nullptr;
    } catch (const std::exception&) {
        set_exception(error);
        return nullptr;
    } catch (...) {
        set_exception(error);
        return nullptr;
    }
}

void impl_kala_shape_free(void* handle) noexcept {
    delete as_shape(handle);
}

int32_t impl_kala_shape_is_null(const void* handle) noexcept {
    if (handle == nullptr) {
        return 1;
    }
    return as_shape(handle)->IsNull() ? 1 : 0;
}

int64_t impl_kala_shape_solid_count(const void* handle, KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr) {
        set_error(error, kInvalidArgument, "shape");
        return -1;
    }
    try {
        const TopoDS_Shape& shape = *as_shape(handle);
        TopExp_Explorer explorer(shape, TopAbs_SOLID);
        int64_t count = 0;
        while (explorer.More()) {
            ++count;
            explorer.Next();
        }
        return count;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return -1;
    } catch (const std::exception&) {
        set_exception(error);
        return -1;
    } catch (...) {
        set_exception(error);
        return -1;
    }
}

// ---------------------------------------------------------------------------
// Measurement
// ---------------------------------------------------------------------------

int32_t impl_kala_shape_is_valid(const void* handle, int32_t* out,
                                 KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr || out == nullptr) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        const TopoDS_Shape& shape = *as_shape(handle);
        *out = BRepCheck_Analyzer(shape).IsValid() ? 1 : 0;
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_shape_publish_state(const void* handle, KalaPublishState* out,
                                 KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr || out == nullptr) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        const TopoDS_Shape& shape = *as_shape(handle);
        if (shape.IsNull()) {
            out->solid_count = 0;
            out->volume = 0.0;
            out->is_null = 1;
            out->valid = 0;
            return kOk;
        }
        TopExp_Explorer explorer(shape, TopAbs_SOLID);
        int64_t count = 0;
        while (explorer.More()) {
            ++count;
            explorer.Next();
        }
        GProp_GProps props;
        BRepGProp::VolumeProperties(shape, props, Standard_False, Standard_False);
        out->solid_count = count;
        out->volume = props.Mass();
        out->is_null = 0;
        out->valid = BRepCheck_Analyzer(shape).IsValid() ? 1 : 0;
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_shape_topology(const void* handle, KalaTopology* out,
                                 KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr || out == nullptr) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        const TopoDS_Shape& shape = *as_shape(handle);
        auto count = [&shape](TopAbs_ShapeEnum kind) -> int64_t {
            TopTools_IndexedMapOfShape map;
            TopExp::MapShapes(shape, kind, map);
            return static_cast<int64_t>(map.Extent());
        };
        out->solids = count(TopAbs_SOLID);
        out->shells = count(TopAbs_SHELL);
        out->faces = count(TopAbs_FACE);
        out->edges = count(TopAbs_EDGE);
        out->vertices = count(TopAbs_VERTEX);
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_shape_volume(const void* handle, double* out, KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr || out == nullptr) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        GProp_GProps props;
        BRepGProp::VolumeProperties(*as_shape(handle), props, Standard_False, Standard_False);
        *out = props.Mass();
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_shape_measure(const void* handle, KalaMeasurement* out,
                           KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr || out == nullptr) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        const TopoDS_Shape& shape = *as_shape(handle);
        if (shape.IsNull()) {
            set_error(error, kInvalidArgument, "null shape");
            return kInvalidArgument;
        }

        GProp_GProps volume_props;
        BRepGProp::VolumeProperties(shape, volume_props, Standard_False, Standard_False);
        GProp_GProps surface_props;
        BRepGProp::SurfaceProperties(shape, surface_props, Standard_False, Standard_False);

        Bnd_Box box;
        box.SetGap(0.0);
        BRepBndLib::AddOptimal(shape, box, Standard_True, Standard_False);
        if (box.IsVoid()) {
            return kVoidBox;
        }
        const gp_Pnt minimum = box.CornerMin();
        const gp_Pnt maximum = box.CornerMax();
        if (!finite3(minimum.X(), minimum.Y(), minimum.Z()) ||
            !finite3(maximum.X(), maximum.Y(), maximum.Z())) {
            return kVoidBox;
        }

        const gp_Pnt centre = volume_props.CentreOfMass();
        TopExp_Explorer explorer(shape, TopAbs_SOLID);
        int64_t count = 0;
        while (explorer.More()) {
            ++count;
            explorer.Next();
        }

        out->volume = volume_props.Mass();
        out->area = surface_props.Mass();
        out->centroid[0] = centre.X();
        out->centroid[1] = centre.Y();
        out->centroid[2] = centre.Z();
        out->min[0] = minimum.X();
        out->min[1] = minimum.Y();
        out->min[2] = minimum.Z();
        out->max[0] = maximum.X();
        out->max[1] = maximum.Y();
        out->max[2] = maximum.Z();
        out->solid_count = count;
        out->valid = BRepCheck_Analyzer(shape).IsValid() ? 1 : 0;
        out->reserved = 0;
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_optimal_box_ex(const void* handle, int32_t use_triangulation,
                                 double* out, KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr || out == nullptr) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        Bnd_Box box;
        box.SetGap(0.0);
        BRepBndLib::AddOptimal(*as_shape(handle), box,
                               use_triangulation != 0 ? Standard_True : Standard_False,
                               Standard_False);
        if (box.IsVoid()) {
            return kVoidBox;
        }
        const gp_Pnt minimum = box.CornerMin();
        const gp_Pnt maximum = box.CornerMax();
        if (!finite3(minimum.X(), minimum.Y(), minimum.Z()) ||
            !finite3(maximum.X(), maximum.Y(), maximum.Z())) {
            return kVoidBox;
        }
        out[0] = minimum.X();
        out[1] = minimum.Y();
        out[2] = minimum.Z();
        out[3] = maximum.X();
        out[4] = maximum.Y();
        out[5] = maximum.Z();
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_optimal_box(const void* handle, double* out, KalaCadError* error) noexcept {
    return impl_kala_optimal_box_ex(handle, 1, out, error);
}

// ---------------------------------------------------------------------------
// Profile faces and solids
// ---------------------------------------------------------------------------

int32_t impl_kala_polygon_face(const double* points, int32_t count, void** out,
                          KalaCadError* error) noexcept {
    clear_error(error);
    if (points == nullptr || out == nullptr || count < 3) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        BRepBuilderAPI_MakePolygon polygon;
        for (int32_t index = 0; index < count; ++index) {
            polygon.Add(gp_Pnt(points[index * 3], points[index * 3 + 1],
                               points[index * 3 + 2]));
        }
        polygon.Close();
        if (!polygon.IsDone()) {
            return kNotDone;
        }
        BRepBuilderAPI_MakeFace face(polygon.Wire());
        if (!face.IsDone()) {
            return kNullResult;
        }
        *out = new_shape(face.Face());
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_circle_face(double cx, double cy, double cz, double nx, double ny,
                         double nz, double radius, void** out,
                         KalaCadError* error) noexcept {
    clear_error(error);
    if (out == nullptr || !finite3(cx, cy, cz) || !finite3(nx, ny, nz) ||
        !std::isfinite(radius) || radius <= 0.0 ||
        (nx * nx + ny * ny + nz * nz) <= 0.0) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        const gp_Ax2 axis(gp_Pnt(cx, cy, cz), gp_Dir(nx, ny, nz));
        const gp_Circ circle(axis, radius);
        BRepBuilderAPI_MakeEdge edge(circle);
        if (!edge.IsDone()) {
            return kNotDone;
        }
        BRepBuilderAPI_MakeWire wire(edge.Edge());
        if (!wire.IsDone()) {
            return kNullResult;
        }
        BRepBuilderAPI_MakeFace face(wire.Wire());
        if (!face.IsDone()) {
            return kNoTriangles;
        }
        *out = new_shape(face.Face());
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_prism(const void* face, double dx, double dy, double dz, void** out,
                   KalaCadError* error) noexcept {
    clear_error(error);
    if (face == nullptr || out == nullptr || !finite3(dx, dy, dz)) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        BRepPrimAPI_MakePrism prism(*as_shape(face), gp_Vec(dx, dy, dz));
        if (!prism.IsDone()) {
            return kNotDone;
        }
        const TopoDS_Shape shape = prism.Shape();
        if (shape.IsNull()) {
            return kNullResult;
        }
        *out = new_shape(shape);
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_revolve_y(const void* face, double radians, void** out,
                       KalaCadError* error) noexcept {
    clear_error(error);
    if (face == nullptr || out == nullptr || !std::isfinite(radians) ||
        radians <= 0.0) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        const gp_Ax1 axis(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 1.0, 0.0));
        BRepPrimAPI_MakeRevol revolve(*as_shape(face), axis, radians, Standard_True);
        if (!revolve.IsDone()) {
            return kNotDone;
        }
        *out = new_shape(revolve.Shape());
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_boolean_cut(const void* left, const void* right, void** out,
                         KalaCadError* error) noexcept {
    clear_error(error);
    if (left == nullptr || right == nullptr || out == nullptr) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        BRepAlgoAPI_Cut operation(*as_shape(left), *as_shape(right));
        if (!operation.IsDone()) {
            return kNotDone;
        }
        const TopoDS_Shape shape = operation.Shape();
        if (shape.IsNull()) {
            return kNullResult;
        }
        *out = new_shape(shape);
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_boolean_fuse(const void* left, const void* right, void** out,
                          KalaCadError* error) noexcept {
    clear_error(error);
    if (left == nullptr || right == nullptr || out == nullptr) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        BRepAlgoAPI_Fuse operation(*as_shape(left), *as_shape(right));
        if (!operation.IsDone()) {
            return kNotDone;
        }
        const TopoDS_Shape shape = operation.Shape();
        if (shape.IsNull()) {
            return kNullResult;
        }
        *out = new_shape(shape);
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_compound(const void* const* shapes, size_t count, void** out,
                      KalaCadError* error) noexcept {
    clear_error(error);
    if (out == nullptr || (count > 0 && shapes == nullptr)) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        TopoDS_Compound compound;
        BRep_Builder builder;
        builder.MakeCompound(compound);
        for (size_t index = 0; index < count; ++index) {
            if (shapes[index] == nullptr) {
                set_error(error, kInvalidArgument, "arguments");
                return kInvalidArgument;
            }
            builder.Add(compound, *as_shape(shapes[index]));
        }
        *out = new_shape(compound);
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_translate(const void* handle, double dx, double dy, double dz,
                       void** out, KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr || out == nullptr || !finite3(dx, dy, dz)) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        gp_Trsf transform;
        transform.SetTranslation(gp_Vec(dx, dy, dz));
        BRepBuilderAPI_Transform builder(*as_shape(handle), transform, Standard_True);
        if (!builder.IsDone()) {
            return kNotDone;
        }
        *out = new_shape(builder.Shape());
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

// ---------------------------------------------------------------------------
// Tessellation
// ---------------------------------------------------------------------------

void impl_kala_mesh_free(KalaMesh* mesh) noexcept {
    delete mesh;
}

int64_t impl_kala_mesh_vertex_count(const KalaMesh* mesh) noexcept {
    return mesh != nullptr ? static_cast<int64_t>(mesh->positions.size() / 3) : -1;
}

int64_t impl_kala_mesh_triangle_count(const KalaMesh* mesh) noexcept {
    return mesh != nullptr ? static_cast<int64_t>(mesh->indices.size() / 3) : -1;
}

int32_t impl_kala_mesh_copy(const KalaMesh* mesh, double* positions,
                       size_t positions_len, double* normals, size_t normals_len,
                       uint32_t* indices, size_t indices_len,
                       KalaCadError* error) noexcept {
    clear_error(error);
    if (mesh == nullptr || positions == nullptr || normals == nullptr ||
        indices == nullptr || positions_len < mesh->positions.size() ||
        normals_len < mesh->normals.size() || indices_len < mesh->indices.size()) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    if (!mesh->positions.empty()) {
        std::memcpy(positions, mesh->positions.data(),
                    mesh->positions.size() * sizeof(double));
    }
    if (!mesh->normals.empty()) {
        std::memcpy(normals, mesh->normals.data(),
                    mesh->normals.size() * sizeof(double));
    }
    if (!mesh->indices.empty()) {
        std::memcpy(indices, mesh->indices.data(),
                    mesh->indices.size() * sizeof(uint32_t));
    }
    return kOk;
}

KalaMesh* impl_kala_mesh_create(const void* handle, double deflection,
                           double angular_deflection, int32_t parallel,
                           int64_t max_vertices, int64_t max_triangles,
                           KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr || !std::isfinite(deflection) || deflection <= 0.0 ||
        !std::isfinite(angular_deflection) || angular_deflection <= 0.0 ||
        max_vertices < 0 || max_triangles < 0) {
        set_error(error, kInvalidArgument, "arguments");
        return nullptr;
    }
    try {
        const TopoDS_Shape& shape = *as_shape(handle);
        BRepMesh_IncrementalMesh mesher(shape, deflection, Standard_False,
                                        angular_deflection, parallel != 0);
        if (!mesher.IsDone()) {
            set_error(error, kNotDone, "not done");
            return nullptr;
        }

        // RAII: every early return and every exception path below releases the
        // partially built mesh; ownership only leaves this function on success.
        std::unique_ptr<KalaMesh> mesh(new KalaMesh());
        TopLoc_Location location;
        TopExp_Explorer explorer(shape, TopAbs_FACE);
        while (explorer.More()) {
            const TopoDS_Face face = TopoDS::Face(explorer.Current());
            const Handle(Poly_Triangulation)& triangulation =
                BRep_Tool::Triangulation(face, location);
            if (triangulation.IsNull()) {
                set_error(error, kNullResult, "no triangulation");
                return nullptr;
            }
            if (!triangulation->HasNormals()) {
                try {
                    triangulation->ComputeNormals();
                } catch (const Standard_Failure& failure) {
                    set_error(error, kNormalsFailed, exception_class(failure));
                    return nullptr;
                } catch (const std::exception&) {
                    set_error(error, kNormalsFailed, "std::exception");
                    return nullptr;
                } catch (...) {
                    set_error(error, kNormalsFailed, "unknown");
                    return nullptr;
                }
            }

            const int64_t node_count = triangulation->NbNodes();
            const int64_t triangle_count = triangulation->NbTriangles();
            const int64_t current_vertices =
                static_cast<int64_t>(mesh->positions.size() / 3);
            const int64_t current_triangles =
                static_cast<int64_t>(mesh->indices.size() / 3);
            if (current_vertices + node_count > max_vertices) {
                set_error(error, kOverVertexBudget, "vertex budget");
                return nullptr;
            }
            if (current_triangles + triangle_count > max_triangles) {
                set_error(error, kOverTriangleBudget, "triangle budget");
                return nullptr;
            }

            const gp_Trsf transform = location.Transformation();
            const bool reversed = face.Orientation() == TopAbs_REVERSED;
            const uint32_t offset = static_cast<uint32_t>(current_vertices);
            for (int64_t node = 1; node <= node_count; ++node) {
                const gp_Pnt point = triangulation->Node(static_cast<int>(node)).Transformed(transform);
                mesh->positions.push_back(point.X());
                mesh->positions.push_back(point.Y());
                mesh->positions.push_back(point.Z());

                const gp_Dir normal = triangulation->Normal(static_cast<int>(node));
                double nx = normal.X();
                double ny = normal.Y();
                double nz = normal.Z();
                if (reversed) {
                    nx = -nx;
                    ny = -ny;
                    nz = -nz;
                }
                const double magnitude = std::sqrt(nx * nx + ny * ny + nz * nz);
                if (magnitude > 0.0) {
                    nx /= magnitude;
                    ny /= magnitude;
                    nz /= magnitude;
                }
                mesh->normals.push_back(nx);
                mesh->normals.push_back(ny);
                mesh->normals.push_back(nz);
            }

            for (int64_t triangle = 1; triangle <= triangle_count; ++triangle) {
                int first = 0;
                int second = 0;
                int third = 0;
                triangulation->Triangle(static_cast<int>(triangle)).Get(first, second, third);
                if (reversed) {
                    const int swap = second;
                    second = third;
                    third = swap;
                }
                mesh->indices.push_back(offset + static_cast<uint32_t>(first - 1));
                mesh->indices.push_back(offset + static_cast<uint32_t>(second - 1));
                mesh->indices.push_back(offset + static_cast<uint32_t>(third - 1));
            }
            explorer.Next();
        }

        if (mesh->indices.empty()) {
            set_error(error, kNoTriangles, "no triangles");
            return nullptr;
        }
        return mesh.release();
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return nullptr;
    } catch (const std::exception&) {
        set_exception(error);
        return nullptr;
    } catch (...) {
        set_exception(error);
        return nullptr;
    }
}

// ---------------------------------------------------------------------------
// STEP in-memory I/O
// ---------------------------------------------------------------------------

void impl_kala_buffer_free(KalaBuffer* buffer) noexcept {
    delete buffer;
}

int64_t impl_kala_buffer_size(const KalaBuffer* buffer) noexcept {
    return buffer != nullptr ? static_cast<int64_t>(buffer->data.size()) : -1;
}

int32_t impl_kala_buffer_copy(const KalaBuffer* buffer, uint8_t* destination,
                         size_t capacity, KalaCadError* error) noexcept {
    clear_error(error);
    if (buffer == nullptr || destination == nullptr ||
        capacity < buffer->data.size()) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    if (!buffer->data.empty()) {
        std::memcpy(destination, buffer->data.data(), buffer->data.size());
    }
    return kOk;
}

KalaBuffer* impl_kala_step_export(const void* handle, KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr) {
        set_error(error, kInvalidArgument, "arguments");
        return nullptr;
    }
    try {
        if (!STEPControl_Controller::Init()) {
            set_error(error, kInvalidArgument, "step controller");
            return nullptr;
        }
        if (!Interface_Static::SetCVal("write.step.unit", "MM")) {
            set_error(error, kInvalidArgument, "write.step.unit");
            return nullptr;
        }
        STEPControl_Writer writer;
        const IFSelect_ReturnStatus transfer =
            writer.Transfer(*as_shape(handle), STEPControl_AsIs);
        if (transfer != IFSelect_RetDone) {
            set_error(error, kNotDone, "transfer");
            return nullptr;
        }
        std::ostringstream stream;
        const IFSelect_ReturnStatus written = writer.WriteStream(stream);
        if (written != IFSelect_RetDone) {
            set_error(error, kNullResult, "write");
            return nullptr;
        }
        // RAII: a throwing std::string assignment releases the buffer.
        std::unique_ptr<KalaBuffer> buffer(new KalaBuffer());
        buffer->data = stream.str();
        return buffer.release();
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return nullptr;
    } catch (const std::exception&) {
        set_exception(error);
        return nullptr;
    } catch (...) {
        set_exception(error);
        return nullptr;
    }
}

void* impl_kala_step_import(const uint8_t* data, size_t length,
                       KalaCadError* error) noexcept {
    clear_error(error);
    if (data == nullptr || length == 0) {
        set_error(error, kInvalidArgument, "arguments");
        return nullptr;
    }
    try {
        if (!STEPControl_Controller::Init()) {
            set_error(error, kInvalidArgument, "step controller");
            return nullptr;
        }
        if (!Interface_Static::SetCVal("xstep.cascade.unit", "MM")) {
            set_error(error, kInvalidArgument, "xstep.cascade.unit");
            return nullptr;
        }
        const std::string text(reinterpret_cast<const char*>(data), length);
        std::istringstream stream(text);
        STEPControl_Reader reader;
        const IFSelect_ReturnStatus read =
            reader.ReadStream("kala.step", stream);
        if (read != IFSelect_RetDone) {
            set_error(error, kNotDone, "read");
            return nullptr;
        }
        const int roots = reader.NbRootsForTransfer();
        if (roots <= 0) {
            set_error(error, kNullResult, "no roots");
            return nullptr;
        }
        const int transferred = reader.TransferRoots();
        if (transferred < 1) {
            set_error(error, kNoTriangles, "no shapes");
            return nullptr;
        }
        const TopoDS_Shape shape = reader.OneShape();
        if (shape.IsNull()) {
            set_error(error, kVoidBox, "null shape");
            return nullptr;
        }
        return new_shape(shape);
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return nullptr;
    } catch (const std::exception&) {
        set_exception(error);
        return nullptr;
    } catch (...) {
        set_exception(error);
        return nullptr;
    }
}

// ---------------------------------------------------------------------------
// Hidden-line drawing and section geometry
// ---------------------------------------------------------------------------

bool finite_cad_direction(const double* values) {
    return values != nullptr && finite3(values[0], values[1], values[2]) &&
           (values[0] * values[0] + values[1] * values[1] + values[2] * values[2]) > 0.0;
}

void append_edges(const TopoDS_Shape& compound, std::vector<TopoDS_Edge>& edges) {
    if (compound.IsNull()) {
        return;
    }
    TopExp_Explorer explorer(compound, TopAbs_EDGE);
    while (explorer.More()) {
        edges.push_back(TopoDS::Edge(explorer.Current()));
        explorer.Next();
    }
}

// Sample one edge into view-plane millimetres. The first and last samples are
// pinned to the exact end points of the curve (the point caps); a closed curve
// repeats its first point at the end. A curve that collapsed to a point yields
// a one-point polyline instead of being dropped.
KalaPolyline discretise_edge(const TopoDS_Edge& edge, const PlaneProjection& projection,
                             double chord_tolerance) {
    KalaPolyline polyline;
    BRepAdaptor_Curve adaptor(edge);
    double first = adaptor.FirstParameter();
    double last = adaptor.LastParameter();
    if (last < first) {
        std::swap(first, last);
    }
    if (last - first <= kGeometryEpsilon) {
        double u = 0.0;
        double v = 0.0;
        project_point(projection, adaptor.Value(first), u, v);
        polyline.coordinates.push_back(u);
        polyline.coordinates.push_back(v);
        return polyline;
    }

    GCPnts_QuasiUniformDeflection sampler;
    sampler.Initialize(adaptor, chord_tolerance, first, last);
    const int count = sampler.NbPoints();
    if (count < 2) {
        double u = 0.0;
        double v = 0.0;
        project_point(projection, adaptor.Value(first), u, v);
        polyline.coordinates.push_back(u);
        polyline.coordinates.push_back(v);
        project_point(projection, adaptor.Value(last), u, v);
        polyline.coordinates.push_back(u);
        polyline.coordinates.push_back(v);
        return polyline;
    }

    for (int index = 1; index <= count; ++index) {
        double u = 0.0;
        double v = 0.0;
        project_point(projection, adaptor.Value(sampler.Parameter(index)), u, v);
        polyline.coordinates.push_back(u);
        polyline.coordinates.push_back(v);
    }
    const std::size_t size = polyline.coordinates.size();
    double start_u = 0.0;
    double start_v = 0.0;
    double end_u = 0.0;
    double end_v = 0.0;
    project_point(projection, adaptor.Value(first), start_u, start_v);
    project_point(projection, adaptor.Value(last), end_u, end_v);
    polyline.coordinates[0] = start_u;
    polyline.coordinates[1] = start_v;
    polyline.coordinates[size - 2] = end_u;
    polyline.coordinates[size - 1] = end_v;
    polyline.closed = adaptor.IsClosed() != 0;
    if (polyline.closed &&
        std::hypot(polyline.coordinates[size - 2] - start_u,
                   polyline.coordinates[size - 1] - start_v) > kGeometryEpsilon) {
        polyline.coordinates.push_back(start_u);
        polyline.coordinates.push_back(start_v);
    }
    return polyline;
}

std::vector<KalaPolyline> polylines_of(const std::vector<TopoDS_Edge>& edges,
                                       const PlaneProjection& projection,
                                       double chord_tolerance) {
    std::vector<KalaPolyline> polylines;
    polylines.reserve(edges.size());
    for (const TopoDS_Edge& edge : edges) {
        polylines.push_back(discretise_edge(edge, projection, chord_tolerance));
    }
    return polylines;
}

// The exact length of the edges of a wire, measured by the kernel.
double wire_length(const TopoDS_Wire& wire) {
    double total = 0.0;
    BRepTools_WireExplorer explorer(wire);
    while (explorer.More()) {
        GProp_GProps properties;
        BRepGProp::LinearProperties(explorer.Current(), properties, Standard_False,
                                    Standard_False);
        total += std::abs(properties.Mass());
        explorer.Next();
    }
    return total;
}

// True when the wire walks its boundary once. A point that starts two edges
// means the wire touches itself, so the face the kernel builds from it is not
// the region the wire bounds.
bool wire_is_simple(const TopoDS_Wire& wire) {
    std::vector<std::tuple<double, double, double>> starts;
    BRepTools_WireExplorer explorer(wire);
    while (explorer.More()) {
        const TopoDS_Edge& edge = explorer.Current();
        TopoDS_Vertex vertex;
        if (edge.Orientation() == TopAbs_REVERSED) {
            vertex = TopExp::LastVertex(edge, Standard_False);
        } else {
            vertex = TopExp::FirstVertex(edge, Standard_False);
        }
        if (vertex.IsNull()) {
            return false;
        }
        const gp_Pnt point = BRep_Tool::Pnt(vertex);
        if (!std::isfinite(point.X()) || !std::isfinite(point.Y()) ||
            !std::isfinite(point.Z())) {
            // Sorting non-finite coordinates would violate the strict weak
            // ordering; a non-finite vertex is never a simple loop.
            return false;
        }
        starts.emplace_back(point.X(), point.Y(), point.Z());
        explorer.Next();
    }
    std::sort(starts.begin(), starts.end());
    for (std::size_t index = 1; index < starts.size(); ++index) {
        const auto& first = starts[index - 1];
        const auto& second = starts[index];
        if (std::abs(std::get<0>(first) - std::get<0>(second)) <= kGeometryEpsilon &&
            std::abs(std::get<1>(first) - std::get<1>(second)) <= kGeometryEpsilon &&
            std::abs(std::get<2>(first) - std::get<2>(second)) <= kGeometryEpsilon) {
            return false;
        }
    }
    return true;
}

std::vector<double> wire_points(const TopoDS_Wire& wire,
                                const PlaneProjection& projection, double chord_tolerance) {
    std::vector<double> points;
    BRepTools_WireExplorer explorer(wire);
    while (explorer.More()) {
        const KalaPolyline polyline =
            discretise_edge(explorer.Current(), projection, chord_tolerance);
        std::vector<double> segment = polyline.coordinates;
        if (explorer.Orientation() == TopAbs_REVERSED) {
            std::vector<double> reversed;
            reversed.reserve(segment.size());
            for (std::size_t index = segment.size(); index >= 2; index -= 2) {
                reversed.push_back(segment[index - 2]);
                reversed.push_back(segment[index - 1]);
            }
            segment.swap(reversed);
        }
        if (!points.empty() && !segment.empty() &&
            std::hypot(points[points.size() - 2] - segment[0],
                       points[points.size() - 1] - segment[1]) <= kGeometryEpsilon) {
            segment.erase(segment.begin(), segment.begin() + 2);
        }
        points.insert(points.end(), segment.begin(), segment.end());
        explorer.Next();
    }
    if (points.size() >= 4 &&
        std::hypot(points[0] - points[points.size() - 2],
                   points[1] - points[points.size() - 1]) <= kGeometryEpsilon) {
        points.resize(points.size() - 2);
    }
    return points;
}

// Measure the exact planar face of a simple wire. Returns false when the
// kernel cannot build the face or the area is not a finite positive number.
bool measured_face_area(const TopoDS_Wire& wire, double& area) {
    BRepBuilderAPI_MakeFace builder(wire, Standard_True);
    if (!builder.IsDone()) {
        return false;
    }
    const TopoDS_Face face = builder.Face();
    if (face.IsNull()) {
        return false;
    }
    GProp_GProps properties;
    BRepGProp::SurfaceProperties(face, properties, Standard_False, Standard_False);
    const double value = std::abs(properties.Mass());
    if (!std::isfinite(value) || value <= 0.0) {
        return false;
    }
    area = value;
    return true;
}

TopoDS_Wire oriented_hole(const TopoDS_Wire& wire, const TopoDS_Wire& outer) {
    if (wire.Orientation() == outer.Orientation()) {
        return TopoDS::Wire(wire.Reversed());
    }
    return wire;
}

int32_t impl_kala_hlr_views(const void* handle, const double* direction, const double* u_axis,
                            double tolerance, void** out, KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr || out == nullptr || !finite_cad_direction(direction) ||
        !finite_cad_direction(u_axis) || !std::isfinite(tolerance) || tolerance <= 0.0) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        const gp_Ax2 axis(gp_Pnt(0.0, 0.0, 0.0),
                          gp_Dir(direction[0], direction[1], direction[2]),
                          gp_Dir(u_axis[0], u_axis[1], u_axis[2]));
        Handle(HLRBRep_Algo) algorithm = new HLRBRep_Algo();
        algorithm->Add(*as_shape(handle));
        const HLRAlgo_Projector projector(axis);
        algorithm->Projector(projector);
        algorithm->Update();
        algorithm->Hide();
        HLRBRep_HLRToShape shapes(algorithm);

        std::vector<TopoDS_Edge> visible_edges;
        append_edges(shapes.VCompound(), visible_edges);
        append_edges(shapes.OutLineVCompound(), visible_edges);
        std::vector<TopoDS_Edge> hidden_edges;
        append_edges(shapes.HCompound(), hidden_edges);
        append_edges(shapes.OutLineHCompound(), hidden_edges);

        // HLRToShape returns its edges in the local frame of the projection
        // plane, so the first two coordinates already are view millimetres.
        const PlaneProjection local{{1.0, 0.0, 0.0}, {0.0, 1.0, 0.0}};
        std::unique_ptr<KalaDrawing> drawing(new KalaDrawing());
        drawing->visible = polylines_of(visible_edges, local, tolerance);
        drawing->hidden = polylines_of(hidden_edges, local, tolerance);
        *out = drawing.release();
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

const std::vector<KalaPolyline>* drawing_lines(const void* drawing, int32_t hidden) {
    if (drawing == nullptr) {
        return nullptr;
    }
    const KalaDrawing& value = *static_cast<const KalaDrawing*>(drawing);
    return hidden != 0 ? &value.hidden : &value.visible;
}

void impl_kala_drawing_free(void* drawing) noexcept {
    delete static_cast<KalaDrawing*>(drawing);
}

int64_t impl_kala_drawing_line_count(const void* drawing, int32_t hidden) noexcept {
    const std::vector<KalaPolyline>* lines = drawing_lines(drawing, hidden);
    return lines != nullptr ? static_cast<int64_t>(lines->size()) : -1;
}

int64_t impl_kala_drawing_point_count(const void* drawing, int32_t hidden,
                                      int64_t line) noexcept {
    const std::vector<KalaPolyline>* lines = drawing_lines(drawing, hidden);
    if (lines == nullptr || line < 0 || static_cast<std::size_t>(line) >= lines->size()) {
        return -1;
    }
    return static_cast<int64_t>((*lines)[static_cast<std::size_t>(line)].coordinates.size() / 2);
}

int32_t impl_kala_drawing_is_closed(const void* drawing, int32_t hidden,
                                    int64_t line) noexcept {
    const std::vector<KalaPolyline>* lines = drawing_lines(drawing, hidden);
    if (lines == nullptr || line < 0 || static_cast<std::size_t>(line) >= lines->size()) {
        return -1;
    }
    return (*lines)[static_cast<std::size_t>(line)].closed ? 1 : 0;
}

int32_t impl_kala_drawing_copy_points(const void* drawing, int32_t hidden, int64_t line,
                                      double* destination, size_t capacity,
                                      KalaCadError* error) noexcept {
    clear_error(error);
    const std::vector<KalaPolyline>* lines = drawing_lines(drawing, hidden);
    if (lines == nullptr || line < 0 || destination == nullptr ||
        static_cast<std::size_t>(line) >= lines->size()) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    const std::vector<double>& coordinates = (*lines)[static_cast<std::size_t>(line)].coordinates;
    if (capacity < coordinates.size()) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    if (!coordinates.empty()) {
        std::memcpy(destination, coordinates.data(), coordinates.size() * sizeof(double));
    }
    return kOk;
}

int32_t impl_kala_shape_y_midpoint(const void* handle, double* out,
                                   KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr || out == nullptr) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        Bnd_Box box;
        BRepBndLib::AddOptimal(*as_shape(handle), box, Standard_True, Standard_False);
        if (!box.HasFinitePart()) {
            return kVoidBox;
        }
        const gp_Pnt minimum = box.CornerMin();
        const gp_Pnt maximum = box.CornerMax();
        if (!std::isfinite(minimum.Y()) || !std::isfinite(maximum.Y())) {
            return kVoidBox;
        }
        *out = (minimum.Y() + maximum.Y()) / 2.0;
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

int32_t impl_kala_section_create(const void* handle, const double* normal, double offset,
                                 const double* u_axis, const double* v_axis, double tolerance,
                                 void** out, KalaCadError* error) noexcept {
    clear_error(error);
    if (handle == nullptr || out == nullptr || !finite_cad_direction(normal) ||
        !finite_cad_direction(u_axis) || !finite_cad_direction(v_axis) ||
        !std::isfinite(offset) || !std::isfinite(tolerance) || tolerance <= 0.0) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        const gp_Pnt origin(normal[0] * offset, normal[1] * offset, normal[2] * offset);
        const gp_Pln plane(origin, gp_Dir(normal[0], normal[1], normal[2]));
        BRepAlgoAPI_Section operation(*as_shape(handle), plane);
        operation.Build();
        if (!operation.IsDone()) {
            set_error(error, kNotDone, "not done");
            return kNotDone;
        }

        std::vector<TopoDS_Edge> edges;
        append_edges(operation.Shape(), edges);
        std::unique_ptr<KalaSection> section(new KalaSection());
        if (edges.empty()) {
            *out = section.release();
            return kOk;
        }

        Handle(TopTools_HSequenceOfShape) sequence = new TopTools_HSequenceOfShape();
        for (const TopoDS_Edge& edge : edges) {
            sequence->Append(edge);
        }
        Handle(TopTools_HSequenceOfShape) wires = new TopTools_HSequenceOfShape();
        ShapeAnalysis_FreeBounds::ConnectEdgesToWires(sequence, kSectionWireTolerance,
                                                      Standard_False, wires);

        const PlaneProjection projection{
            {u_axis[0], u_axis[1], u_axis[2]},
            {v_axis[0], v_axis[1], v_axis[2]},
        };
        for (int index = 1; index <= wires->Size(); ++index) {
            KalaLoop loop;
            loop.wire = TopoDS::Wire(wires->Value(index));
            loop.coordinates = wire_points(loop.wire, projection, tolerance);
            if (loop.coordinates.size() < 6) {
                // A loop needs at least three plane points to enclose an area.
                continue;
            }
            loop.length = wire_length(loop.wire);
            loop.simple = wire_is_simple(loop.wire);
            if (loop.simple) {
                loop.area_measured = measured_face_area(loop.wire, loop.area);
            }
            section->loops.push_back(std::move(loop));
        }
        *out = section.release();
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

void impl_kala_section_free(void* section) noexcept {
    delete static_cast<KalaSection*>(section);
}

const KalaLoop* section_loop(const void* section, int64_t index) {
    if (section == nullptr || index < 0) {
        return nullptr;
    }
    const KalaSection& value = *static_cast<const KalaSection*>(section);
    if (static_cast<std::size_t>(index) >= value.loops.size()) {
        return nullptr;
    }
    return &value.loops[static_cast<std::size_t>(index)];
}

int64_t impl_kala_section_loop_count(const void* section) noexcept {
    if (section == nullptr) {
        return -1;
    }
    return static_cast<int64_t>(static_cast<const KalaSection*>(section)->loops.size());
}

int32_t impl_kala_section_loop_info(const void* section, int64_t index,
                                    KalaSectionLoopInfo* out, KalaCadError* error) noexcept {
    clear_error(error);
    const KalaLoop* loop = section_loop(section, index);
    if (loop == nullptr || out == nullptr) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    out->point_count = static_cast<int64_t>(loop->coordinates.size() / 2);
    out->length = loop->length;
    out->area = loop->area;
    out->area_measured = loop->area_measured ? 1 : 0;
    out->simple = loop->simple ? 1 : 0;
    out->reserved = 0;
    return kOk;
}

int32_t impl_kala_section_loop_copy_points(const void* section, int64_t index,
                                           double* destination, size_t capacity,
                                           KalaCadError* error) noexcept {
    clear_error(error);
    const KalaLoop* loop = section_loop(section, index);
    if (loop == nullptr || destination == nullptr ||
        capacity < loop->coordinates.size()) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    if (!loop->coordinates.empty()) {
        std::memcpy(destination, loop->coordinates.data(),
                    loop->coordinates.size() * sizeof(double));
    }
    return kOk;
}

int32_t impl_kala_section_net_area(const void* section, int64_t outer,
                                   const int64_t* holes, size_t hole_count, double* out,
                                   KalaCadError* error) noexcept {
    clear_error(error);
    const KalaLoop* region = section_loop(section, outer);
    if (region == nullptr || out == nullptr || (hole_count > 0 && holes == nullptr)) {
        set_error(error, kInvalidArgument, "arguments");
        return kInvalidArgument;
    }
    try {
        BRepBuilderAPI_MakeFace builder(region->wire, Standard_True);
        if (!builder.IsDone()) {
            return kNotDone;
        }
        for (size_t index = 0; index < hole_count; ++index) {
            const KalaLoop* hole = section_loop(section, holes[index]);
            if (hole == nullptr) {
                set_error(error, kInvalidArgument, "arguments");
                return kInvalidArgument;
            }
            builder.Add(oriented_hole(hole->wire, region->wire));
        }
        const TopoDS_Face face = builder.Face();
        if (face.IsNull()) {
            return kNotDone;
        }
        GProp_GProps properties;
        BRepGProp::SurfaceProperties(face, properties, Standard_False, Standard_False);
        const double area = std::abs(properties.Mass());
        if (!std::isfinite(area) || area <= 0.0) {
            return kNotDone;
        }
        *out = area;
        return kOk;
    } catch (const Standard_Failure& failure) {
        set_error(error, kException, exception_class(failure));
        return kException;
    } catch (const std::exception&) {
        set_exception(error);
        return kException;
    } catch (...) {
        set_exception(error);
        return kException;
    }
}

}  // namespace

extern "C" {

const char* kala_occt_version(void) noexcept { return impl_kala_occt_version(); }

void* kala_shape_new_null(KalaCadError* error) noexcept {
    return impl_kala_shape_new_null(error);
}
void* kala_shape_copy(const void* handle, KalaCadError* error) noexcept {
    return impl_kala_shape_copy(handle, error);
}
void kala_shape_free(void* handle) noexcept { impl_kala_shape_free(handle); }
int32_t kala_shape_is_null(const void* handle) noexcept {
    return impl_kala_shape_is_null(handle);
}
int64_t kala_shape_solid_count(const void* handle, KalaCadError* error) noexcept {
    return impl_kala_shape_solid_count(handle, error);
}
int32_t kala_shape_is_valid(const void* handle, int32_t* out,
                            KalaCadError* error) noexcept {
    return impl_kala_shape_is_valid(handle, out, error);
}
int32_t kala_shape_publish_state(const void* handle, KalaPublishState* out,
                                 KalaCadError* error) noexcept {
    return impl_kala_shape_publish_state(handle, out, error);
}
int32_t kala_shape_topology(const void* handle, KalaTopology* out,
                            KalaCadError* error) noexcept {
    return impl_kala_shape_topology(handle, out, error);
}
int32_t kala_shape_volume(const void* handle, double* out,
                          KalaCadError* error) noexcept {
    return impl_kala_shape_volume(handle, out, error);
}
int32_t kala_shape_measure(const void* handle, KalaMeasurement* out,
                           KalaCadError* error) noexcept {
    return impl_kala_shape_measure(handle, out, error);
}
int32_t kala_optimal_box(const void* handle, double* out,
                         KalaCadError* error) noexcept {
    return impl_kala_optimal_box(handle, out, error);
}
int32_t kala_optimal_box_ex(const void* handle, int32_t use_triangulation,
                            double* out, KalaCadError* error) noexcept {
    return impl_kala_optimal_box_ex(handle, use_triangulation, out, error);
}
int32_t kala_polygon_face(const double* points, int32_t count, void** out,
                          KalaCadError* error) noexcept {
    return impl_kala_polygon_face(points, count, out, error);
}
int32_t kala_circle_face(double cx, double cy, double cz, double nx, double ny,
                         double nz, double radius, void** out,
                         KalaCadError* error) noexcept {
    return impl_kala_circle_face(cx, cy, cz, nx, ny, nz, radius, out, error);
}
int32_t kala_prism(const void* face, double dx, double dy, double dz, void** out,
                   KalaCadError* error) noexcept {
    return impl_kala_prism(face, dx, dy, dz, out, error);
}
int32_t kala_revolve_y(const void* face, double radians, void** out,
                       KalaCadError* error) noexcept {
    return impl_kala_revolve_y(face, radians, out, error);
}
int32_t kala_boolean_cut(const void* left, const void* right, void** out,
                         KalaCadError* error) noexcept {
    return impl_kala_boolean_cut(left, right, out, error);
}
int32_t kala_boolean_fuse(const void* left, const void* right, void** out,
                          KalaCadError* error) noexcept {
    return impl_kala_boolean_fuse(left, right, out, error);
}
int32_t kala_compound(const void* const* shapes, size_t count, void** out,
                      KalaCadError* error) noexcept {
    return impl_kala_compound(shapes, count, out, error);
}
int32_t kala_translate(const void* handle, double dx, double dy, double dz,
                       void** out, KalaCadError* error) noexcept {
    return impl_kala_translate(handle, dx, dy, dz, out, error);
}
void kala_mesh_free(KalaMesh* mesh) noexcept { impl_kala_mesh_free(mesh); }
int64_t kala_mesh_vertex_count(const KalaMesh* mesh) noexcept {
    return impl_kala_mesh_vertex_count(mesh);
}
int64_t kala_mesh_triangle_count(const KalaMesh* mesh) noexcept {
    return impl_kala_mesh_triangle_count(mesh);
}
int32_t kala_mesh_copy(const KalaMesh* mesh, double* positions,
                       size_t positions_len, double* normals, size_t normals_len,
                       uint32_t* indices, size_t indices_len,
                       KalaCadError* error) noexcept {
    return impl_kala_mesh_copy(mesh, positions, positions_len, normals, normals_len,
                            indices, indices_len, error);
}
KalaMesh* kala_mesh_create(const void* handle, double deflection,
                           double angular_deflection, int32_t parallel,
                           int64_t max_vertices, int64_t max_triangles,
                           KalaCadError* error) noexcept {
    return impl_kala_mesh_create(handle, deflection, angular_deflection, parallel,
                              max_vertices, max_triangles, error);
}
void kala_buffer_free(KalaBuffer* buffer) noexcept {
    impl_kala_buffer_free(buffer);
}
int64_t kala_buffer_size(const KalaBuffer* buffer) noexcept {
    return impl_kala_buffer_size(buffer);
}
int32_t kala_buffer_copy(const KalaBuffer* buffer, uint8_t* destination,
                         size_t capacity, KalaCadError* error) noexcept {
    return impl_kala_buffer_copy(buffer, destination, capacity, error);
}
KalaBuffer* kala_step_export(const void* handle, KalaCadError* error) noexcept {
    return impl_kala_step_export(handle, error);
}
void* kala_step_import(const uint8_t* data, size_t length,
                       KalaCadError* error) noexcept {
    return impl_kala_step_import(data, length, error);
}
int32_t kala_shape_y_midpoint(const void* handle, double* out,
                              KalaCadError* error) noexcept {
    return impl_kala_shape_y_midpoint(handle, out, error);
}
int32_t kala_hlr_views(const void* handle, const double* direction, const double* u_axis,
                       double tolerance, void** out, KalaCadError* error) noexcept {
    return impl_kala_hlr_views(handle, direction, u_axis, tolerance, out, error);
}
void kala_drawing_free(void* drawing) noexcept { impl_kala_drawing_free(drawing); }
int64_t kala_drawing_line_count(const void* drawing, int32_t hidden) noexcept {
    return impl_kala_drawing_line_count(drawing, hidden);
}
int64_t kala_drawing_point_count(const void* drawing, int32_t hidden,
                                 int64_t line) noexcept {
    return impl_kala_drawing_point_count(drawing, hidden, line);
}
int32_t kala_drawing_is_closed(const void* drawing, int32_t hidden,
                               int64_t line) noexcept {
    return impl_kala_drawing_is_closed(drawing, hidden, line);
}
int32_t kala_drawing_copy_points(const void* drawing, int32_t hidden, int64_t line,
                                 double* destination, size_t capacity,
                                 KalaCadError* error) noexcept {
    return impl_kala_drawing_copy_points(drawing, hidden, line, destination, capacity,
                                         error);
}
int32_t kala_section_create(const void* handle, const double* normal, double offset,
                            const double* u_axis, const double* v_axis, double tolerance,
                            void** out, KalaCadError* error) noexcept {
    return impl_kala_section_create(handle, normal, offset, u_axis, v_axis, tolerance, out,
                                    error);
}
void kala_section_free(void* section) noexcept { impl_kala_section_free(section); }
int64_t kala_section_loop_count(const void* section) noexcept {
    return impl_kala_section_loop_count(section);
}
int32_t kala_section_loop_info(const void* section, int64_t index,
                               KalaSectionLoopInfo* out,
                               KalaCadError* error) noexcept {
    return impl_kala_section_loop_info(section, index, out, error);
}
int32_t kala_section_loop_copy_points(const void* section, int64_t index,
                                      double* destination, size_t capacity,
                                      KalaCadError* error) noexcept {
    return impl_kala_section_loop_copy_points(section, index, destination, capacity, error);
}
int32_t kala_section_net_area(const void* section, int64_t outer, const int64_t* holes,
                              size_t hole_count, double* out,
                              KalaCadError* error) noexcept {
    return impl_kala_section_net_area(section, outer, holes, hole_count, out, error);
}

}  // extern "C"
